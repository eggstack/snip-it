#!/bin/bash
set -e

echo "Building snip-sync server..."
cargo build --release

echo "Removing old binary..."
rm -f snip-sync

echo "Copying new binary..."
cp target/release/snip-sync .

echo "Done! Binary available at: $(pwd)/snip-sync"
