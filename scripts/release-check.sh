#!/usr/bin/env bash
# scripts/release-check.sh — exhaustive pre-release verification.
#
# Usage:
#   bash scripts/release-check.sh verify          Full local correctness and packaging validation
#   bash scripts/release-check.sh dry-run <crate>  Validate one crate for publishing (dry-run)
#
# The verify mode requires a clean working tree. The dry-run mode
# accepts only snip-proto, snip-sync, or snip-it.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

usage() {
    echo "Usage:"
    echo "  bash scripts/release-check.sh verify          Full local verification"
    echo "  bash scripts/release-check.sh dry-run <crate>  Per-crate publish dry-run"
    echo ""
    echo "Accepted crate names: snip-proto, snip-sync, snip-it"
    exit 2
}

require_clean_tree() {
    if ! git diff --quiet HEAD 2>/dev/null; then
        echo "ERROR: Working tree is not clean. Commit or stash changes first."
        exit 1
    fi
    if ! git diff --cached --quiet 2>/dev/null; then
        echo "ERROR: Staged changes exist. Commit or unstage them first."
        exit 1
    fi
}

run_verify() {
    require_clean_tree

    echo "=== Phase 1: Focused checks ==="
    bash "$SCRIPT_DIR/check.sh"

    echo ""
    echo "=== Phase 2: Deep integration tests ==="
    cargo test --workspace --all-features -- --test-threads=1

    echo ""
    echo "=== Phase 3: Release build ==="
    cargo build --workspace --release --all-features

    echo ""
    echo "=== Phase 4: Release-profile crash and contract tests ==="
    cargo test --release --test cleanup_crash_failpoints \
      --features test-support -- --test-threads=1
    cargo test --release --test restore_crash_failpoints \
      --features test-support -- --test-threads=1
    cargo test --release --test deterministic_e2e \
      --features test-support -- --test-threads=1
    cargo test --release --test transaction_crash_recovery \
      --features test-support -- --test-threads=1

    echo ""
    echo "=== Phase 5: Production seam proof ==="
    bash "$SCRIPT_DIR/ci/test-production-seams.sh"

    echo ""
    echo "=== Phase 6: Package validation ==="
    cargo package -p snip-proto --locked
    cargo package -p snip-sync --locked
    cargo package -p snip-it --locked

    echo ""
    echo "=== All release checks passed ==="
}

run_dry_run() {
    local crate="$1"

    case "$crate" in
        snip-proto|snip-sync|snip-it) ;;
        *)
            echo "ERROR: Unknown crate '$crate'. Accepted: snip-proto, snip-sync, snip-it"
            exit 2
            ;;
    esac

    require_clean_tree

    echo "=== Dry-run publish for $crate ==="
    cargo publish -p "$crate" --dry-run --locked
    echo ""
    echo "=== Dry-run passed for $crate ==="
    echo ""
    echo "To publish manually:"
    echo "  cargo publish -p $crate"
}

# --- Main ---
if [ $# -lt 1 ]; then
    usage
fi

case "$1" in
    verify)
        run_verify
        ;;
    dry-run)
        if [ $# -lt 2 ]; then
            echo "ERROR: dry-run requires a crate name"
            usage
        fi
        run_dry_run "$2"
        ;;
    *)
        echo "ERROR: Unknown command '$1'"
        usage
        ;;
esac
