#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALLER="$SCRIPT_DIR/../../packaging/install.sh"
# shellcheck source=/dev/null
source "$INSTALLER"

assert_eq() {
    local expected="$1"
    local actual="$2"
    local label="$3"
    [[ "$expected" == "$actual" ]] || {
        echo "FAIL: $label: expected '$expected', got '$actual'" >&2
        exit 1
    }
}

assert_contains() {
    local haystack="$1"
    local needle="$2"
    local label="$3"
    [[ "$haystack" == *"$needle"* ]] || {
        echo "FAIL: $label: expected to contain '$needle', got '$haystack'" >&2
        exit 1
    }
}

# ---------------------------------------------------------------------------
# Pure mapping / construction / argument contract (Plan 002 baseline)
# ---------------------------------------------------------------------------

assert_eq 'x86_64-unknown-linux-gnu' "$(target_for_unix Linux x86_64)" 'Linux x86_64 mapping'
assert_eq 'aarch64-unknown-linux-gnu' "$(target_for_unix Linux aarch64)" 'Linux ARM64 mapping'
assert_eq 'armv7-unknown-linux-gnueabihf' "$(target_for_unix Linux armv7l)" 'Linux ARMv7 mapping'
assert_eq 'x86_64-apple-darwin' "$(target_for_unix Darwin x86_64)" 'Intel macOS mapping'
assert_eq 'aarch64-apple-darwin' "$(target_for_unix Darwin arm64)" 'Apple Silicon mapping'
assert_eq 'source-only' "$(target_for_unix FreeBSD x86_64)" 'source-only mapping'
assert_eq 'v1.2.3' "$(component_tag snp 1.2.3)" 'snp release tag'
assert_eq 'snip-sync-v0.1.4' "$(component_tag server 0.1.4)" 'snip-sync release tag'
assert_eq 'snp-x86_64-unknown-linux-gnu' "$(asset_filename snp x86_64-unknown-linux-gnu)" 'snp asset'
assert_eq 'snip-sync-aarch64-unknown-linux-gnu' "$(asset_filename server aarch64-unknown-linux-gnu)" 'server asset'
assert_eq 'http://example.test/releases/v1.2.3/snp-x86_64-unknown-linux-gnu' \
    "$(SNP_INSTALL_TEST_MODE=1 SNP_INSTALL_GITHUB_BASE=http://example.test/releases \
        release_asset_url snp 1.2.3 x86_64-unknown-linux-gnu)" 'snp asset URL'
assert_eq 'http://example.test/releases/snip-sync-v0.1.4/snip-sync-x86_64-unknown-linux-gnu' \
    "$(SNP_INSTALL_TEST_MODE=1 SNP_INSTALL_GITHUB_BASE=http://example.test/releases \
        release_asset_url server 0.1.4 x86_64-unknown-linux-gnu)" 'snip-sync asset URL'
# Independent tags for the same numeric version prove --both cannot reuse one tag.
assert_eq 'v1.2.3' "$(component_tag snp 1.2.3)" 'snp tag independence'
assert_eq 'snip-sync-v1.2.3' "$(component_tag server 1.2.3)" 'server tag independence'
assert_eq '/usr/local/bin' "$(destination_dir_for_identity 0 /home/alice)" 'system destination'
assert_eq '/home/alice/.local/bin' "$(destination_dir_for_identity 1000 /home/alice)" 'user destination'

# source-only classification (Plan 002: ARMv7 + unknown hosts go to Cargo).
source_only_target 'source-only'
source_only_target 'armv7-unknown-linux-gnueabihf'
if source_only_target 'x86_64-unknown-linux-gnu'; then
    echo 'FAIL: prebuilt Linux x86_64 classified as source-only' >&2
    exit 1
fi
if source_only_target 'aarch64-apple-darwin'; then
    echo 'FAIL: prebuilt macOS ARM64 classified as source-only' >&2
    exit 1
fi

INSTALL_COMPONENTS=()
INSTALL_VERSION=''
parse_args --both
assert_eq '2' "${#INSTALL_COMPONENTS[@]}" '--both component count'
assert_eq '' "$INSTALL_VERSION" '--both has independent versions'
assert_eq 'snp' "${INSTALL_COMPONENTS[0]}" '--both first component'
assert_eq 'server' "${INSTALL_COMPONENTS[1]}" '--both second component'

if parse_args --both --version 1.2.3; then
    echo 'FAIL: --both accepted an ambiguous --version' >&2
    exit 1
fi
if parse_args --version 1.2; then
    echo 'FAIL: invalid stable version accepted' >&2
    exit 1
fi

export PATH="/usr/bin:/home/alice/.local/bin"
path_contains /home/alice/.local/bin
if path_contains /opt/bin; then
    echo 'FAIL: unrelated PATH entry reported as present' >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# Fixture HTTP server (Python stdlib only, no new dependencies)
# ---------------------------------------------------------------------------

FIXTURE_ROOT="$(mktemp -d)"
SERVE_ROOT="$FIXTURE_ROOT/serve"
mkdir -p "$SERVE_ROOT"
FIXTURE_SERVER_PY="$FIXTURE_ROOT/fixture_server.py"
cat > "$FIXTURE_SERVER_PY" <<'PY'
import http.server
import json
import os
import urllib.parse

SERVE_ROOT = os.environ["FIXTURE_SERVE_ROOT"]

class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        path = urllib.parse.urlparse(self.path).path
        if path.startswith("/crates/"):
            pkg = path.rsplit("/", 1)[-1]
            versions = {"snip-it": "1.2.3", "snip-sync": "0.9.9"}
            ver = versions.get(pkg, "1.2.3")
            body = json.dumps({"crate": {"max_stable_version": ver, "max_version": ver}}).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        if path.startswith("/releases/"):
            rel = path[len("/releases/"):]
            # Inject a deterministic 500 for the dedicated failure version so
            # install_component can prove non-404 transport failures are hard
            # failures without Cargo fallback.
            if rel.startswith("v9.9.9/") or rel.startswith("snip-sync-v9.9.9/"):
                self.send_response(500)
                self.send_header("Content-Length", "5")
                self.end_headers()
                self.wfile.write(b"error")
                return
            # Prevent directory traversal.
            parts = [p for p in rel.split("/") if p not in ("", ".", "..")]
            fpath = os.path.join(SERVE_ROOT, *parts) if parts else SERVE_ROOT
            if os.path.isfile(fpath):
                with open(fpath, "rb") as handle:
                    data = handle.read()
                self.send_response(200)
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)
                return
            self.send_response(404)
            self.send_header("Content-Length", "9")
            self.end_headers()
            self.wfile.write(b"not found")
            return
        self.send_response(404)
        self.send_header("Content-Length", "9")
        self.end_headers()
        self.wfile.write(b"not found")

    def log_message(self, *args):
        pass

http.server.ThreadingHTTPServer(("127.0.0.1", int(os.environ["FIXTURE_PORT"])), Handler).serve_forever()
PY

free_port() {
    python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'
}

FIXTURE_PORT="$(free_port)"
export FIXTURE_SERVE_ROOT="$SERVE_ROOT"
export FIXTURE_PORT
export SNP_INSTALL_TEST_MODE=1
export SNP_INSTALL_GITHUB_BASE="http://127.0.0.1:$FIXTURE_PORT/releases"
export SNP_INSTALL_CRATES_API_BASE="http://127.0.0.1:$FIXTURE_PORT"

python3 "$FIXTURE_SERVER_PY" >/dev/null 2>&1 &
FIXTURE_PID=$!
cleanup_fixture() {
    kill "$FIXTURE_PID" 2>/dev/null || true
    rm -rf "$FIXTURE_ROOT"
}
trap cleanup_fixture EXIT

# Wait for the fixture server to accept connections.
for _ in $(seq 1 50); do
    if curl --silent --output /dev/null "http://127.0.0.1:$FIXTURE_PORT/crates/snip-it"; then
        break
    fi
    sleep 0.1
done
curl --silent --output /dev/null "http://127.0.0.1:$FIXTURE_PORT/crates/snip-it" || {
    echo 'FAIL: fixture HTTP server did not start' >&2
    exit 1
}

# crates.io seam resolves each component independently.
assert_eq '1.2.3' "$(crate_version snip-it)" 'crates seam snp version'
assert_eq '0.9.9' "$(crate_version snip-sync)" 'crates seam server version'

# download_http boundaries: success, 404, non-404, transport failure.
TMP_DL="$(mktemp -d)"
echo -n 'payload' > "$SERVE_ROOT/payload.bin"
mkdir -p "$SERVE_ROOT/v1.2.3"
cp "$SERVE_ROOT/payload.bin" "$SERVE_ROOT/v1.2.3/snp-x86_64-unknown-linux-gnu"
download_http "http://127.0.0.1:$FIXTURE_PORT/releases/v1.2.3/snp-x86_64-unknown-linux-gnu" "$TMP_DL/out.bin" || {
    echo 'FAIL: download_http success path failed' >&2
    exit 1
}
assert_eq 'payload' "$(cat "$TMP_DL/out.bin")" 'download_http body'
if download_http "http://127.0.0.1:$FIXTURE_PORT/releases/v0.0.0/missing-asset" "$TMP_DL/missing.bin"; then
    echo 'FAIL: download_http 404 unexpectedly succeeded' >&2
    exit 1
else
    assert_eq '44' "$?" 'download_http 404 status'
fi
if download_http "http://127.0.0.1:$FIXTURE_PORT/releases/v9.9.9/snp-x86_64-unknown-linux-gnu" "$TMP_DL/fail.bin"; then
    echo 'FAIL: download_http 500 unexpectedly succeeded' >&2
    exit 1
else
    status=$?
    [[ "$status" != '44' ]] || {
        echo 'FAIL: download_http 500 collapsed into 404 fallback status' >&2
        exit 1
    }
fi
if download_http "http://127.0.0.1:1/unroutable" "$TMP_DL/transport.bin"; then
    echo 'FAIL: download_http transport failure unexpectedly succeeded' >&2
    exit 1
else
    status=$?
    [[ "$status" != '44' && "$status" != '0' ]] || {
        echo 'FAIL: transport failure must not select Cargo fallback status' >&2
        exit 1
    }
fi

# ---------------------------------------------------------------------------
# verify_candidate boundaries (direct, no Cargo involved)
# ---------------------------------------------------------------------------

make_fake_binary() {
    local path="$1"
    local identity="$2"
    printf '#!/usr/bin/env bash\necho "%s"\n' "$identity" > "$path"
    chmod +x "$path"
}

VERIFY_DIR="$(mktemp -d)"
CURRENT_COMPONENT='snp'
CURRENT_VERSION='1.2.3'
ASSET='snp-x86_64-unknown-linux-gnu'
CANDIDATE="$VERIFY_DIR/$ASSET"
make_fake_binary "$CANDIDATE" 'snp 1.2.3'
DIGEST="$(sha256sum "$CANDIDATE" | awk '{print $1}')"
printf '%s  %s\n' "$DIGEST" "$ASSET" > "$VERIFY_DIR/$ASSET.sha256"
verify_candidate "$CANDIDATE" "$VERIFY_DIR/$ASSET.sha256" "$ASSET"

# Malformed checksum sidecar -> hard failure.
printf 'not-a-checksum\n' > "$VERIFY_DIR/bad.sha256"
if verify_candidate "$CANDIDATE" "$VERIFY_DIR/bad.sha256" "$ASSET"; then
    echo 'FAIL: malformed checksum accepted' >&2
    exit 1
fi
printf '%s  %s extra-field\n' "$DIGEST" "$ASSET" > "$VERIFY_DIR/extra.sha256"
if verify_candidate "$CANDIDATE" "$VERIFY_DIR/extra.sha256" "$ASSET"; then
    echo 'FAIL: checksum with extra field accepted' >&2
    exit 1
fi
printf 'deadbeef  wrong-name\n' > "$VERIFY_DIR/wrongname.sha256"
if verify_candidate "$CANDIDATE" "$VERIFY_DIR/wrongname.sha256" "$ASSET"; then
    echo 'FAIL: checksum with wrong asset name accepted' >&2
    exit 1
fi

# SHA-256 mismatch -> hard failure.
printf '%s  %s\n' '0000000000000000000000000000000000000000000000000000000000000000' "$ASSET" > "$VERIFY_DIR/mismatch.sha256"
if verify_candidate "$CANDIDATE" "$VERIFY_DIR/mismatch.sha256" "$ASSET"; then
    echo 'FAIL: SHA-256 mismatch accepted' >&2
    exit 1
fi

# Wrong program identity -> hard failure.
WRONG_ID="$VERIFY_DIR/wrong-id"
make_fake_binary "$WRONG_ID" 'snip-sync 1.2.3'
WRONG_DIGEST="$(sha256sum "$WRONG_ID" | awk '{print $1}')"
printf '%s  %s\n' "$WRONG_DIGEST" "$ASSET" > "$VERIFY_DIR/wrong-id.sha256"
if verify_candidate "$WRONG_ID" "$VERIFY_DIR/wrong-id.sha256" "$ASSET"; then
    echo 'FAIL: wrong program identity accepted' >&2
    exit 1
fi

# Wrong version -> hard failure.
WRONG_VER="$VERIFY_DIR/wrong-ver"
make_fake_binary "$WRONG_VER" 'snp 9.9.9'
WRONG_VER_DIGEST="$(sha256sum "$WRONG_VER" | awk '{print $1}')"
printf '%s  %s\n' "$WRONG_VER_DIGEST" "$ASSET" > "$VERIFY_DIR/wrong-ver.sha256"
if verify_candidate "$WRONG_VER" "$VERIFY_DIR/wrong-ver.sha256" "$ASSET"; then
    echo 'FAIL: wrong candidate version accepted' >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# install_component boundaries with stubbed Cargo fallback
# ---------------------------------------------------------------------------

# Isolate HOME so install_component never touches the developer's config.
TEST_HOME="$(mktemp -d)"
export HOME="$TEST_HOME"
export XDG_CONFIG_HOME="$TEST_HOME/.config"
# Use the real user-local destination shape ($HOME/.local/bin) so the PATH
# warning branch in install_component is exercised faithfully.
TEST_DEST="$TEST_HOME/.local/bin"
mkdir -p "$TEST_DEST"
destination_dir() {
    printf '%s\n' "$TEST_DEST"
}
# Avoid real server post-install side effects during component tests.
post_install_server() {
    return 0
}

CARGO_MARKER="$FIXTURE_ROOT/cargo-fallback-called"
reset_cargo_marker() {
    rm -f "$CARGO_MARKER"
}
# Stub the fallback decision at the smallest practical seam: prove the branch
# taken without compiling a second copy of the project.
build_cargo_candidate() {
    local component="$1"
    local version="$2"
    local root="$3"
    touch "$CARGO_MARKER"
    local package binary candidate
    package="$(component_package "$component")"
    binary="$(component_binary "$component")"
    mkdir -p "$root/bin"
    candidate="$root/bin/$binary"
    printf '#!/usr/bin/env bash\necho "%s %s"\n' "$binary" "$version" > "$candidate"
    chmod +x "$candidate"
    CURRENT_COMPONENT="$component"
    CURRENT_VERSION="$version"
    verify_binary_identity "$candidate" "$binary"
    printf '%s\n' "$candidate"
}

# Keep required tooling on PATH; destination starts off PATH so the PATH
# warning behavior can be asserted deterministically.
export PATH="/usr/bin:/bin:/usr/local/bin"

TARGET_UNDER_TEST="$(target_for_unix "$(uname -s)" "$(uname -m)")"
if source_only_target "$TARGET_UNDER_TEST"; then
    echo "SKIP: fixture host target '$TARGET_UNDER_TEST' is source-only; prebuilt failure-mode tests require a prebuilt host" >&2
    exit 1
fi
HOST_ASSET="snp-$TARGET_UNDER_TEST"

setup_snp_fixture() {
    local mode="$1"
    local tag_dir="$SERVE_ROOT/v1.2.3"
    rm -rf "$tag_dir"
    mkdir -p "$tag_dir"
    local asset="$HOST_ASSET"
    local candidate="$FIXTURE_ROOT/candidate-$mode"
    case "$mode" in
        valid)
            make_fake_binary "$candidate" 'snp 1.2.3'
            cp "$candidate" "$tag_dir/$asset"
            (cd "$FIXTURE_ROOT" && sha256sum "candidate-$mode" | awk -v a="$asset" '{print $1"  "a}' > "$tag_dir/$asset.sha256")
            ;;
        mismatch)
            make_fake_binary "$candidate" 'snp 1.2.3'
            cp "$candidate" "$tag_dir/$asset"
            printf '%s  %s\n' 'ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff' "$asset" > "$tag_dir/$asset.sha256"
            ;;
        malformed)
            make_fake_binary "$candidate" 'snp 1.2.3'
            cp "$candidate" "$tag_dir/$asset"
            printf 'malformed-sidecar\n' > "$tag_dir/$asset.sha256"
            ;;
        missing-checksum)
            make_fake_binary "$candidate" 'snp 1.2.3'
            cp "$candidate" "$tag_dir/$asset"
            ;;
        wrong-identity)
            make_fake_binary "$candidate" 'snip-sync 1.2.3'
            cp "$candidate" "$tag_dir/$asset"
            (cd "$FIXTURE_ROOT" && sha256sum "candidate-$mode" | awk -v a="$asset" '{print $1"  "a}' > "$tag_dir/$asset.sha256")
            ;;
        wrong-version)
            make_fake_binary "$candidate" 'snp 0.0.0'
            cp "$candidate" "$tag_dir/$asset"
            (cd "$FIXTURE_ROOT" && sha256sum "candidate-$mode" | awk -v a="$asset" '{print $1"  "a}' > "$tag_dir/$asset.sha256")
            ;;
        notfound)
            # No asset files: fixture server returns 404.
            ;;
        *)
            echo "FAIL: unknown fixture mode '$mode'" >&2
            exit 1
            ;;
    esac
}

# Valid candidate + checksum installs into the isolated destination without
# Cargo fallback and prints PATH guidance when the destination is off PATH.
setup_snp_fixture valid
reset_cargo_marker
rm -f "$TEST_DEST/snp"
install_output="$(install_component snp 1.2.3 2>&1)"
assert_contains "$install_output" 'Installed snp 1.2.3' 'valid install output'
assert_contains "$install_output" "Add $TEST_DEST to PATH" 'PATH warning when off PATH'
[[ -x "$TEST_DEST/snp" ]] || {
    echo 'FAIL: valid install did not place executable' >&2
    exit 1
}
assert_eq 'snp 1.2.3' "$("$TEST_DEST/snp" version)" 'installed identity'
[[ ! -f "$CARGO_MARKER" ]] || {
    echo 'FAIL: valid prebuilt install invoked Cargo fallback' >&2
    exit 1
}

# Destination on PATH suppresses the warning.
export PATH="$TEST_DEST:/usr/bin:/bin"
setup_snp_fixture valid
reset_cargo_marker
rm -f "$TEST_DEST/snp"
install_output="$(install_component snp 1.2.3 2>&1)"
if [[ "$install_output" == *"Add $TEST_DEST to PATH"* ]]; then
    echo 'FAIL: PATH warning printed while destination is on PATH' >&2
    exit 1
fi
export PATH="/usr/bin:/bin:/usr/local/bin"

# 404 selects exact-version Cargo fallback.
setup_snp_fixture notfound
reset_cargo_marker
rm -f "$TEST_DEST/snp"
install_output="$(install_component snp 1.2.3 2>&1)"
assert_contains "$install_output" 'using Cargo fallback' '404 fallback message'
[[ -f "$CARGO_MARKER" ]] || {
    echo 'FAIL: asset 404 did not attempt Cargo fallback' >&2
    exit 1
}
[[ -x "$TEST_DEST/snp" ]] || {
    echo 'FAIL: 404 fallback did not install stub candidate' >&2
    exit 1
}

# Non-404 server failure is a hard failure with no fallback.
reset_cargo_marker
if install_component snp 9.9.9 >/dev/null 2>&1; then
    echo 'FAIL: non-404 asset failure unexpectedly succeeded' >&2
    exit 1
fi
[[ ! -f "$CARGO_MARKER" ]] || {
    echo 'FAIL: non-404 failure invoked Cargo fallback' >&2
    exit 1
}

# Missing checksum after a successful binary download is a hard failure.
setup_snp_fixture missing-checksum
reset_cargo_marker
if install_component snp 1.2.3 >/dev/null 2>&1; then
    echo 'FAIL: missing checksum unexpectedly succeeded' >&2
    exit 1
fi
[[ ! -f "$CARGO_MARKER" ]] || {
    echo 'FAIL: missing checksum invoked Cargo fallback' >&2
    exit 1
}

# Malformed checksum sidecar is a hard failure.
setup_snp_fixture malformed
reset_cargo_marker
if install_component snp 1.2.3 >/dev/null 2>&1; then
    echo 'FAIL: malformed checksum unexpectedly succeeded' >&2
    exit 1
fi
[[ ! -f "$CARGO_MARKER" ]] || {
    echo 'FAIL: malformed checksum invoked Cargo fallback' >&2
    exit 1
}

# SHA-256 mismatch is a hard failure.
setup_snp_fixture mismatch
reset_cargo_marker
if install_component snp 1.2.3 >/dev/null 2>&1; then
    echo 'FAIL: SHA mismatch unexpectedly succeeded' >&2
    exit 1
fi
[[ ! -f "$CARGO_MARKER" ]] || {
    echo 'FAIL: SHA mismatch invoked Cargo fallback' >&2
    exit 1
}

# Wrong program identity is a hard failure.
setup_snp_fixture wrong-identity
reset_cargo_marker
if install_component snp 1.2.3 >/dev/null 2>&1; then
    echo 'FAIL: wrong identity unexpectedly succeeded' >&2
    exit 1
fi
[[ ! -f "$CARGO_MARKER" ]] || {
    echo 'FAIL: wrong identity invoked Cargo fallback' >&2
    exit 1
}

# Wrong version is a hard failure.
setup_snp_fixture wrong-version
reset_cargo_marker
if install_component snp 1.2.3 >/dev/null 2>&1; then
    echo 'FAIL: wrong version unexpectedly succeeded' >&2
    exit 1
fi
[[ ! -f "$CARGO_MARKER" ]] || {
    echo 'FAIL: wrong version invoked Cargo fallback' >&2
    exit 1
}

# Source-only target selects Cargo fallback without attempting a download.
uname() {
    case "${1:-}" in
        -s) printf '%s\n' 'FreeBSD' ;;
        -m) printf '%s\n' 'x86_64' ;;
        *) command uname "$@" ;;
    esac
}
reset_cargo_marker
rm -f "$TEST_DEST/snp"
install_output="$(install_component snp 1.2.3 2>&1)"
assert_contains "$install_output" 'source-only; using Cargo fallback' 'source-only fallback message'
[[ -f "$CARGO_MARKER" ]] || {
    echo 'FAIL: source-only target did not attempt Cargo fallback' >&2
    exit 1
}
unset -f uname

# --both resolves components independently: main dispatches one
# install_component call per component with no shared pinned version.
INSTALL_DISPATCH_LOG="$FIXTURE_ROOT/dispatch.log"
rm -f "$INSTALL_DISPATCH_LOG"
install_component() {
    printf '%s|%s\n' "$1" "${2:-}" >> "$INSTALL_DISPATCH_LOG"
    return 0
}
INSTALL_COMPONENTS=()
INSTALL_VERSION=''
main --both >/dev/null 2>&1
assert_eq '2' "$(grep -c . "$INSTALL_DISPATCH_LOG")" '--both dispatch count'
assert_eq 'snp|' "$(sed -n '1p' "$INSTALL_DISPATCH_LOG")" '--both snp resolves independently'
assert_eq 'server|' "$(sed -n '2p' "$INSTALL_DISPATCH_LOG")" '--both server resolves independently'

rm -rf "$TMP_DL" "$VERIFY_DIR" "$TEST_HOME"
trap - EXIT
cleanup_fixture
trap cleanup_temp_dirs EXIT 2>/dev/null || true

echo 'installer contract tests passed'
