#!/usr/bin/env bash
# scripts/check.sh — ordinary developer verification before pushing.
set -euo pipefail

echo "=== Format check ==="
cargo fmt --all -- --check

echo "=== Clippy ==="
cargo clippy --workspace --all-targets --all-features -- -D warnings

echo "=== Build ==="
cargo build --workspace --all-features

echo "=== Tests ==="
cargo test --workspace --all-features -- --test-threads=1

echo "=== All checks passed ==="
