#!/usr/bin/env bash
# scripts/release-check.sh — manual pre-release verification.
#
# Usage:
#   bash scripts/release-check.sh verify             Full local correctness and packaging validation
#   bash scripts/release-check.sh tag <tag>          Validate a release tag and print JSON metadata
#   bash scripts/release-check.sh dry-run <crate>    Validate one crate for publishing (dry-run)
#
# The verify mode requires a clean working tree. The dry-run mode
# accepts only snip-proto, snip-sync, or snip-it.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

usage() {
    echo "Usage:"
    echo "  bash scripts/release-check.sh verify             Full local verification"
    echo "  bash scripts/release-check.sh tag <tag>          Validate a release tag and print JSON metadata"
    echo "  bash scripts/release-check.sh dry-run <crate>    Per-crate publish dry-run"
    echo ""
    echo "Accepted crate names: snip-proto, snip-sync, snip-it"
    exit 2
}

release_tag_metadata() {
    local tag="$1"
    local component version package manifest tag_commit head_commit

    case "$tag" in
        v[0-9]*.[0-9]*.[0-9]*)
            if [[ ! "$tag" =~ ^v([0-9]+\.[0-9]+\.[0-9]+)$ ]]; then
                echo "ERROR: Invalid snp release tag '$tag'; expected vX.Y.Z" >&2
                exit 1
            fi
            component="snp"
            package="snip-it"
            manifest="Cargo.toml"
            version="${BASH_REMATCH[1]}"
            ;;
        snip-sync-v[0-9]*.[0-9]*.[0-9]*)
            if [[ ! "$tag" =~ ^snip-sync-v([0-9]+\.[0-9]+\.[0-9]+)$ ]]; then
                echo "ERROR: Invalid snip-sync release tag '$tag'; expected snip-sync-vA.B.C" >&2
                exit 1
            fi
            component="snip-sync"
            package="snip-sync"
            manifest="snip-sync/Cargo.toml"
            version="${BASH_REMATCH[1]}"
            ;;
        *)
            echo "ERROR: Unsupported release tag '$tag'; expected vX.Y.Z or snip-sync-vA.B.C" >&2
            exit 1
            ;;
    esac

    tag_commit="$(git rev-parse --verify "refs/tags/${tag}^{commit}" 2>/dev/null)" || {
        echo "ERROR: Release tag '$tag' is not available locally" >&2
        exit 1
    }
    head_commit="$(git rev-parse --verify HEAD)"
    if [[ "$tag_commit" != "$head_commit" ]]; then
        echo "ERROR: Release tag '$tag' points to $tag_commit, but checkout is $head_commit" >&2
        exit 1
    fi

    manifest_version="$(cargo metadata --manifest-path "$manifest" --no-deps --format-version 1 \
        | jq -r --arg package "$package" '.packages[] | select(.name == $package) | .version' \
        | head -n 1)"
    if [[ -z "$manifest_version" || "$manifest_version" == "null" ]]; then
        echo "ERROR: Could not read package '$package' from $manifest" >&2
        exit 1
    fi
    if [[ "$manifest_version" != "$version" ]]; then
        echo "ERROR: Tag '$tag' requires $manifest version $version, found $manifest_version" >&2
        exit 1
    fi

    jq -cn \
        --arg component "$component" \
        --arg package "$package" \
        --arg version "$version" \
        --arg tag "$tag" \
        --arg manifest "$manifest" \
        '{component: $component, package: $package, version: $version, tag: $tag, manifest: $manifest}'
}

verify_release_contract() {
    local workflow=".github/workflows/release-binaries.yml"
    local required

    [[ -f "$workflow" ]] || {
        echo "ERROR: Missing release workflow: $workflow"
        exit 1
    }
    for required in \
        "workflow_dispatch:" \
        "inputs:" \
        "mode:" \
        "default: verify" \
        "type: choice" \
        "ref: \${{ inputs.tag || github.ref }}" \
        "RELEASE_MODE" \
        "mode == 'attach'" \
        "name: Verify complete release asset set" \
        "ubuntu-24.04-arm" \
        "x86_64-unknown-linux-gnu.2.17" \
        "aarch64-unknown-linux-gnu.2.17" \
        "--release --locked" \
        "sha256" \
        "gh release upload"; do
        if ! grep -Fq -- "$required" "$workflow"; then
            echo "ERROR: Release workflow is missing required contract: $required"
            exit 1
        fi
    done
    if grep -Fq "cargo publish" "$workflow"; then
        echo "ERROR: Release workflow must not publish crates"
        exit 1
    fi
    if grep -Eq 'CRATES_IO|crates\.io.*TOKEN|CARGO_REGISTRY_TOKEN' "$workflow"; then
        echo "ERROR: Release workflow must not contain crates.io credentials"
        exit 1
    fi
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

    echo "=== Release workflow contract ==="
    verify_release_contract

    echo "=== Phase 1: Routine checks (same as Linux CI) ==="
    bash "$SCRIPT_DIR/check.sh"

    echo ""
    echo "=== Phase 2: Release build ==="
    cargo build --workspace --release

    echo ""
    echo "=== Phase 3: Release smoke ==="
    # Client version and help
    cargo run --release --bin snp -- --version
    cargo run --release --bin snp -- --help >/dev/null

    # Crash recovery (release-profile)
    cargo test --release --test transaction_crash_recovery \
      --features test-support -- --test-threads=1

    # Multi-batch sync (release-profile)
    cargo test --release --test sync_multibatch -- --test-threads=1

    # Server lifetime regression
    cargo test --release --test snip_sync_lifetime -- --ignored --test-threads=1

    # Production seam proof
    bash "$SCRIPT_DIR/ci/test-production-seams.sh"

    # Manifest/restore contracts (packaging shape)
    cargo test --test manifest_contracts --features test-support

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
    tag)
        if [ $# -ne 2 ]; then
            echo "ERROR: tag requires exactly one tag argument"
            usage
        fi
        release_tag_metadata "$2"
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
