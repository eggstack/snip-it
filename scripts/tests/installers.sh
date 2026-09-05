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
        release_asset_url snp 1.2.3 x86_64-unknown-linux-gnu)" 'asset URL'
assert_eq '/usr/local/bin' "$(destination_dir_for_identity 0 /home/alice)" 'system destination'
assert_eq '/home/alice/.local/bin' "$(destination_dir_for_identity 1000 /home/alice)" 'user destination'

SNP_INSTALL_COMPONENTS=()
SNP_INSTALL_VERSION=''
parse_args --both
assert_eq '2' "${#INSTALL_COMPONENTS[@]}" '--both component count'
assert_eq '' "$INSTALL_VERSION" '--both has independent versions'

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

echo 'installer contract tests passed'
