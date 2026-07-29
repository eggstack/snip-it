#!/usr/bin/env bash
# scripts/check.sh — focused developer verification before pushing.
#
# This is the authoritative check list used by both local developers
# and Linux CI. It runs fmt, clippy, build, unit tests, and a focused
# set of fast integration tests. Deep crash, restore, and protocol
# suites belong in scripts/release-check.sh.
set -euo pipefail

echo "=== Format check ==="
cargo fmt --all -- --check

echo "=== Clippy ==="
cargo clippy --workspace --all-targets --all-features -- -D warnings

echo "=== Build ==="
cargo build --workspace --all-features

echo "=== Unit tests ==="
cargo test --workspace --all-features --lib -- --test-threads=1

echo "=== Platform smoke ==="
cargo test --test platform_smoke --features test-support -- --test-threads=1

echo "=== Manifest contracts ==="
cargo test --test manifest_contracts --features test-support -- --test-threads=1

echo "=== Destination permissions ==="
cargo test --test destination_permissions --features test-support -- --test-threads=1

echo "=== Executor noop success ==="
cargo test --test executor_noop_success --features test-support -- --test-threads=1

echo "=== All checks passed ==="
