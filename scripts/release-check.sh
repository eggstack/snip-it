#!/usr/bin/env bash
# scripts/release-check.sh — manual pre-release verification.
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
    local status
    status="$(git status --porcelain=v1 --untracked-files=all 2>/dev/null || true)"
    if [[ -n "$status" ]]; then
        echo "ERROR: Working tree is not clean. Commit or stash changes first."
        echo "$status"
        exit 1
    fi
}

run_verify() {
    require_clean_tree

    echo "=== Phase 1: Routine checks (same as Linux CI) ==="
    bash "$SCRIPT_DIR/check.sh"

    echo ""
    echo "=== Phase 2: Release build ==="
    cargo build --workspace --release --all-features

    echo ""
    echo "=== Phase 3: Release smoke ==="
    # Client version and help
    cargo run --release --all-features -- --version
    cargo run --release --all-features -- --help >/dev/null

    # Crash recovery (release-profile)
    cargo test --release --test transaction_crash_recovery \
      --features test-support -- --test-threads=1

    # Multi-batch sync (release-profile)
    cargo test --release --test sync_multibatch -- --test-threads=1

    # Server lifetime regression
    cargo test --release --test snip_sync_lifetime -- --ignored --test-threads=1

    # Production seam proof
    bash "$SCRIPT_DIR/ci/test-production-seams.sh"

    echo ""
    echo "=== Phase 4: Package validation ==="
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
