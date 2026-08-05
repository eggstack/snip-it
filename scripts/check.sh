#!/usr/bin/env bash
# scripts/check.sh — focused developer verification before pushing.
#
# This is the authoritative check list used by both local developers
# and Linux CI. It runs fmt, clippy, unit tests, and a focused set of
# fast integration tests. Deep crash, restore, and protocol suites
# belong in scripts/release-check.sh.
#
# Clippy compiles the workspace, so no standalone `cargo build` is needed.
# Clippy uses default production features; test-only code is linted through
# targets that explicitly enable test-support or test-helpers.
# Unit tests are parallel-safe (each uses an isolated TempDir).
set -euo pipefail

echo "=== Format check ==="
cargo fmt --all -- --check

echo "=== Clippy ==="
cargo clippy --workspace --all-targets -- -D warnings

echo "=== Unit tests ==="
cargo test --workspace --lib

echo "=== Platform smoke ==="
cargo test --test platform_smoke

echo "=== Manifest contracts ==="
cargo test --test manifest_contracts

echo "=== Destination permissions ==="
cargo test --test destination_permissions --features test-support

echo "=== Single-helper auto-sync contracts ==="
cargo test --test auto_sync_closure

echo "=== Multi-batch sync contracts ==="
cargo test --test sync_multibatch -- --test-threads=1

echo "=== All checks passed ==="
