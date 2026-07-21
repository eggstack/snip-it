#!/usr/bin/env bash
# Performance baseline script for Phase 06A.
# Captures binary size, cold-start time, and basic operation latency.
# Run from the project root after `cargo build --release`.
set -euo pipefail

BIN="${1:-target/release/snp}"
OUT_DIR="benches"
mkdir -p "$OUT_DIR"

if [ ! -f "$BIN" ]; then
    echo "Binary not found at $BIN. Run 'cargo build --release' first."
    exit 1
fi

TIMESTAMP=$(date +%Y%m%d_%H%M%S)
REPORT="$OUT_DIR/baseline_${TIMESTAMP}.md"

echo "# Performance Baseline — $(date -Iseconds)" > "$REPORT"
echo "" >> "$REPORT"

# Binary size
SIZE_BYTES=$(stat -f%z "$BIN" 2>/dev/null || stat --format=%s "$BIN" 2>/dev/null)
SIZE_KB=$((SIZE_BYTES / 1024))
echo "## Binary Size" >> "$REPORT"
echo "- **${SIZE_KB} KB** (${SIZE_BYTES} bytes)" >> "$REPORT"
echo "" >> "$REPORT"

# Cold-start time (version command)
echo "## Cold Start" >> "$REPORT"
for i in 1 2 3; do
    START=$(date +%s%N)
    "$BIN" --version > /dev/null 2>&1
    END=$(date +%s%N)
    ELAPSED_MS=$(( (END - START) / 1000000 ))
    echo "- Run $i: ${ELAPSED_MS}ms" >> "$REPORT"
done
echo "" >> "$REPORT"

# List command latency
echo "## List Command" >> "$REPORT"
for i in 1 2 3; do
    START=$(date +%s%N)
    "$BIN" list --format json > /dev/null 2>&1 || true
    END=$(date +%s%N)
    ELAPSED_MS=$(( (END - START) / 1000000 ))
    echo "- Run $i: ${ELAPSED_MS}ms" >> "$REPORT"
done
echo "" >> "$REPORT"

# Status command latency
echo "## Status Command" >> "$REPORT"
for i in 1 2 3; do
    START=$(date +%s%N)
    "$BIN" status --format json > /dev/null 2>&1 || true
    END=$(date +%s%N)
    ELAPSED_MS=$(( (END - START) / 1000000 ))
    echo "- Run $i: ${ELAPSED_MS}ms" >> "$REPORT"
done
echo "" >> "$REPORT"

echo "Baseline saved to $REPORT"
cat "$REPORT"
