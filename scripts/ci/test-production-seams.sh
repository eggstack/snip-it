#!/usr/bin/env bash
# Production seam proof — verifies that a binary built without `test-support`
# cannot be influenced by test-only environment variables.
#
# This script:
# 1. Builds `snp` without `test-support` into an isolated target directory.
# 2. Sets matching valid seam values and runs valid scenarios.
# 3. Asserts that no test behavior activates.
#
# Usage: scripts/ci/test-production-seams.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"

TARGET_DIR="target/production-seam"
BINARY="$TARGET_DIR/release/snp"

echo "=== Building production binary (no test-support) ==="
cargo build --release --no-default-features --target-dir "$TARGET_DIR"

if [ ! -f "$BINARY" ]; then
    echo "FAIL: production binary not found at $BINARY"
    exit 1
fi

# Create a temporary config dir for the test scenario.
TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

CONFIG_HOME="$TMPDIR/config"
mkdir -p "$CONFIG_HOME/snp"
export XDG_CONFIG_HOME="$CONFIG_HOME"
export SNP_ALLOW_PLAINTEXT_API_KEY=true

# Write a minimal sync.toml so the executor has something to load.
cat > "$CONFIG_HOME/snp/sync.toml" <<'TOML'
[settings.sync]
enabled = true
server_url = "http://127.0.0.1:1"
api_key = "test-key"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = false
TOML

echo ""
echo "=== Test 1: SNP_TEST_FAILPOINT does not abort production binary ==="
SNP_TEST_FAILPOINT="restore-after-prepared" \
    "$BINARY" list >/dev/null 2>&1
if [ $? -ne 0 ]; then
    echo "FAIL: production binary aborted or errored with matching failpoint"
    exit 1
fi
echo "PASS: failpoint did not abort production binary"

echo ""
echo "=== Test 2: SNP_TEST_EXECUTOR_MODE does not bypass executor ==="
# The noop-success seam should not exist in production. The executor
# should attempt real sync (and fail to connect), not exit 0 immediately.
set +e
SNP_TEST_EXECUTOR_MODE="noop-success" \
    "$BINARY" auto-sync-execute --state-dir "$TMPDIR/state" >/dev/null 2>&1
EXIT_CODE=$?
set -e
if [ $EXIT_CODE -eq 0 ]; then
    echo "FAIL: production executor exited 0 with noop-success mode (seam is active)"
    exit 1
fi
echo "PASS: noop-success mode did not bypass production executor (exit code: $EXIT_CODE)"

echo ""
echo "=== Test 3: SNP_SKIP_WORKER_SPAWN does not suppress production scheduling ==="
SNP_SKIP_WORKER_SPAWN=1 \
    "$BINARY" list >/dev/null 2>&1
if [ $? -ne 0 ]; then
    echo "FAIL: production binary errored with SNP_SKIP_WORKER_SPAWN set"
    exit 1
fi
echo "PASS: worker spawn suppression did not affect production binary"

echo ""
echo "=== Test 4: SNP_TEST_EVENTS_DIR does not create event files ==="
EVENTS_DIR="$TMPDIR/events"
mkdir -p "$EVENTS_DIR"
SNP_TEST_EVENTS_DIR="$EVENTS_DIR" \
    "$BINARY" list >/dev/null 2>&1
if [ -f "$EVENTS_DIR/test-events.jsonl" ]; then
    echo "FAIL: production binary created event file at $EVENTS_DIR/test-events.jsonl"
    exit 1
fi
echo "PASS: no event file created in production binary"

echo ""
echo "=== Test 5: SNP_TEST_MUTATION_BARRIER_DIR does not block production ==="
BARRIER_DIR="$TMPDIR/barrier"
mkdir -p "$BARRIER_DIR"
echo "snippet-save" > "$BARRIER_DIR/point"
SNP_TEST_MUTATION_BARRIER_DIR="$BARRIER_DIR" \
    "$BINARY" list >/dev/null 2>&1
if [ $? -ne 0 ]; then
    echo "FAIL: production binary blocked or errored with mutation barrier set"
    exit 1
fi
# No "entered" file should exist since production ignores the barrier.
if [ -f "$BARRIER_DIR/entered" ]; then
    echo "FAIL: production binary entered mutation barrier"
    exit 1
fi
echo "PASS: mutation barrier did not block production binary"

echo ""
echo "=== All production seam tests passed ==="
