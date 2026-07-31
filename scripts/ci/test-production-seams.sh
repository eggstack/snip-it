#!/usr/bin/env bash
# Production seam proof — verifies that a binary built without `test-support`
# cannot be influenced by test-only environment variables.
#
# This script:
# 1. Builds `snp` without `test-support` into an isolated target directory.
# 2. Sets matching valid seam values and runs valid scenarios that traverse
#    the guarded code paths.
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

# Helper: bounded wait for a file to appear, returns 0 if found, 1 on timeout.
wait_for_file() {
    local file="$1"
    local timeout_secs="${2:-10}"
    local elapsed=0
    while [ ! -f "$file" ]; do
        sleep 0.2
        elapsed=$((elapsed + 1))
        if [ "$elapsed" -ge $((timeout_secs * 5)) ]; then
            return 1
        fi
    done
    return 0
}

# Helper: bounded wait for a process to exit, returns 0 if exited, 1 on timeout.
wait_for_exit() {
    local pid="$1"
    local timeout_secs="${2:-15}"
    local elapsed=0
    while kill -0 "$pid" 2>/dev/null; do
        sleep 0.5
        elapsed=$((elapsed + 1))
        if [ "$elapsed" -ge $((timeout_secs * 2)) ]; then
            kill -9 "$pid" 2>/dev/null || true
            return 1
        fi
    done
    return 0
}

echo ""
echo "=== Test 1: SNP_TEST_FAILPOINT does not abort production restore ==="
# Create a valid backup to restore.
BACKUP_DIR="$TMPDIR/valid-backup"
mkdir -p "$BACKUP_DIR/libraries"
LIB_CONTENT='[[snippets]]
id = "test-1"
description = "test snippet"
command = "echo test"
'
echo "$LIB_CONTENT" > "$BACKUP_DIR/libraries/default.toml"
INDEX_CONTENT='[[libraries]]
filename = "default"
is_primary = true
'
echo "$INDEX_CONTENT" > "$BACKUP_DIR/libraries.toml"
LIB_SHA=$(sha256sum "$BACKUP_DIR/libraries/default.toml" | awk '{print $1}')
INDEX_SHA=$(sha256sum "$BACKUP_DIR/libraries.toml" | awk '{print $1}')
LIB_SIZE=$(wc -c < "$BACKUP_DIR/libraries/default.toml" | tr -d ' ')
INDEX_SIZE=$(wc -c < "$BACKUP_DIR/libraries.toml" | tr -d ' ')
cat > "$BACKUP_DIR/manifest.toml" <<EOF
schema = 1
created_at_unix_ms = 1700000000000
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "default.toml"
kind = "library"
size = $LIB_SIZE
sha256 = "$LIB_SHA"

[[files]]
path = "libraries.toml"
kind = "index"
size = $INDEX_SIZE
sha256 = "$INDEX_SHA"
EOF

# Run restore with the failpoint env var set. Production binary ignores it.
SNP_TEST_FAILPOINT="restore-after-prepared" \
    "$BINARY" restore "$BACKUP_DIR" --mode dry-run >/dev/null 2>&1
if [ $? -ne 0 ]; then
    echo "FAIL: production binary aborted or errored with matching failpoint"
    exit 1
fi
echo "PASS: failpoint did not abort production restore"

echo ""
echo "=== Test 2: removed executor command is not accepted ==="
if "$BINARY" auto-sync-execute --state-dir "$TMPDIR/state" --generation 1 >/dev/null 2>&1; then
    echo "FAIL: removed auto-sync-execute command is still accepted"
    exit 1
fi
echo "PASS: executor subprocess command is removed"

echo ""
echo "=== Test 3: SNP_SKIP_WORKER_SPAWN does not suppress production scheduling ==="
# Perform a real mutation (create a library) with SNP_SKIP_WORKER_SPAWN set.
# Production binary ignores the variable — the mutation should complete normally.
# We use a reachable-but-non-syncing config (auto_sync=false) so the mutation
# completes without attempting network operations.
SNP_SKIP_WORKER_SPAWN=1 \
    "$BINARY" library create seam-test >/dev/null 2>&1
if [ $? -ne 0 ]; then
    echo "FAIL: production binary errored with SNP_SKIP_WORKER_SPAWN set during real mutation"
    exit 1
fi
# Verify the library file was actually created (mutation succeeded).
if [ ! -f "$CONFIG_HOME/snp/libraries/seam-test.toml" ]; then
    echo "FAIL: library file was not created — mutation was suppressed"
    exit 1
fi
echo "PASS: worker spawn suppression did not affect production mutation"

echo ""
echo "=== Test 4: SNP_TEST_EVENTS_DIR does not create event files ==="
# Run the real helper path with SNP_TEST_EVENTS_DIR set.
# Production binary ignores the variable — no event file should be created.
EVENTS_DIR="$TMPDIR/events"
mkdir -p "$EVENTS_DIR"
SNP_TEST_EVENTS_DIR="$EVENTS_DIR" \
    "$BINARY" auto-sync-worker --state-dir "$TMPDIR/state" \
    >/dev/null 2>&1 || true
if [ -f "$EVENTS_DIR/test-events.jsonl" ]; then
    echo "FAIL: production binary created event file at $EVENTS_DIR/test-events.jsonl"
    exit 1
fi
echo "PASS: no event file created in production binary"

echo ""
echo "=== Test 5: SNP_TEST_MUTATION_BARRIER_DIR does not block production ==="
# Set up a barrier directory for a barrier point reached by library creation.
BARRIER_DIR="$TMPDIR/barrier"
mkdir -p "$BARRIER_DIR"
echo "library-create" > "$BARRIER_DIR/point"
# Do NOT create a release file — if the binary checked the barrier, it would hang.
SNP_TEST_MUTATION_BARRIER_DIR="$BARRIER_DIR" \
    "$BINARY" library create barrier-test >/dev/null 2>&1 &
BARRIER_PID=$!
if ! wait_for_exit "$BARRIER_PID" 10; then
    echo "FAIL: production binary blocked with mutation barrier set (timeout)"
    kill -9 "$BARRIER_PID" 2>/dev/null || true
    exit 1
fi
wait "$BARRIER_PID"
EXIT_CODE=$?
if [ $EXIT_CODE -ne 0 ]; then
    echo "FAIL: production binary errored with mutation barrier set (exit: $EXIT_CODE)"
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
