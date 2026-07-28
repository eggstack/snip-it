#!/usr/bin/env bash
# scripts/release-check.sh — exhaustive pre-release verification.
#
# Run this before publishing to crates.io. It performs deeper checks
# than the ordinary check script, including release-profile builds,
# crash failpoint tests, production seam verification, and package checks.
set -euo pipefail

echo "=== Phase 1: Ordinary checks ==="
bash scripts/check.sh

echo ""
echo "=== Phase 2: Release build ==="
cargo build --workspace --release --all-features

echo ""
echo "=== Phase 3: Release-profile crash and contract tests ==="
cargo test --release --test cleanup_crash_failpoints \
  --features test-support -- --test-threads=1
cargo test --release --test restore_crash_failpoints \
  --features test-support -- --test-threads=1
cargo test --release --test manifest_contracts \
  --features test-support -- --test-threads=1
cargo test --release --test deterministic_e2e \
  --features test-support -- --test-threads=1
cargo test --release --test executor_noop_success \
  --features test-support -- --test-threads=1

echo ""
echo "=== Phase 4: Production seam proof ==="
bash scripts/ci/test-production-seams.sh

echo ""
echo "=== Phase 5: Package validation ==="
cargo package -p snip-proto --locked --allow-dirty
cargo package -p snip-sync --locked --allow-dirty
cargo package -p snip-it --locked --allow-dirty

echo ""
echo "=== All release checks passed ==="
echo ""
echo "To publish (manual, in dependency order):"
echo "  cargo publish -p snip-proto"
echo "  cargo publish -p snip-sync"
echo "  cargo publish -p snip-it"
