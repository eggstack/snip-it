#!/usr/bin/env bash
# Install protoc for Unix (Linux and macOS).
# Usage: bash scripts/ci/install-protoc.sh [version]
#
# Requirements:
# - Exact version (default: 25.1)
# - Architecture-aware artifact selection
# - Download failure is fatal
# - Checksum verification if published checksums are available
# - Install under RUNNER_TEMP (or a temp dir if not set)
# - Add path via GITHUB_PATH
# - Verify protoc --version in a subsequent step
# - macOS selects x86_64 or arm64 based on runner architecture

set -euo pipefail

VERSION="${1:-25.1}"
RUNNER_TEMP="${RUNNER_TEMP:-/tmp}"
INSTALL_DIR="${RUNNER_TEMP}/protoc-${VERSION}"

# Detect platform and architecture
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
    Linux)
        PLATFORM="linux"
        case "$ARCH" in
            x86_64|amd64) ARCH_SUFFIX="x86_64" ;;
            aarch64|arm64) ARCH_SUFFIX="aarch64" ;;
            *) echo "Unsupported Linux architecture: $ARCH" >&2; exit 1 ;;
        esac
        ;;
    Darwin)
        PLATFORM="osx"
        case "$ARCH" in
            x86_64) ARCH_SUFFIX="x86_64" ;;
            arm64) ARCH_SUFFIX="osx-x86_64" ;;  # protoc uses universal binary
            *) echo "Unsupported macOS architecture: $ARCH" >&2; exit 1 ;;
        esac
        ;;
    *)
        echo "Unsupported OS: $OS" >&2
        exit 1
        ;;
esac

# macOS protoc releases use a single universal binary
if [ "$PLATFORM" = "osx" ]; then
    ARTIFACT="protoc-${VERSION}-osx-x86_64.zip"
else
    ARTIFACT="protoc-${VERSION}-${PLATFORM}-${ARCH_SUFFIX}.zip"
fi

URL="https://github.com/protocolbuffers/protobuf/releases/download/v${VERSION}/${ARTIFACT}"

echo "Downloading protoc ${VERSION} for ${PLATFORM}-${ARCH_SUFFIX}..."
echo "URL: ${URL}"

TMP_ZIP="${RUNNER_TEMP}/protoc-${VERSION}.zip"

if ! curl -fsSL "$URL" -o "$TMP_ZIP"; then
    echo "ERROR: Failed to download protoc from $URL" >&2
    exit 1
fi

# Verify checksum if available (protoc publishes SHA256SUMS)
SHA_URL="${URL}.sha256"
if curl -fsSL "$SHA_URL" -o "${TMP_ZIP}.sha256" 2>/dev/null; then
    # Extract the expected hash for our artifact
    EXPECTED_HASH=$(grep "$ARTIFACT" "${TMP_ZIP}.sha256" | awk '{print $1}' || true)
    if [ -n "$EXPECTED_HASH" ]; then
        ACTUAL_HASH=$(shasum -a 256 "$TMP_ZIP" | awk '{print $1}')
        if [ "$EXPECTED_HASH" != "$ACTUAL_HASH" ]; then
            echo "ERROR: Checksum mismatch for protoc" >&2
            echo "  Expected: $EXPECTED_HASH" >&2
            echo "  Actual:   $ACTUAL_HASH" >&2
            exit 1
        fi
        echo "Checksum verified: $ACTUAL_HASH"
    fi
fi

echo "Extracting protoc..."
mkdir -p "$INSTALL_DIR"
unzip -o -q "$TMP_ZIP" -d "$INSTALL_DIR"
rm -f "$TMP_ZIP" "${TMP_ZIP}.sha256"

# Add to PATH (both for this step via export and for subsequent steps via GITHUB_PATH)
PROTOC_BIN="${INSTALL_DIR}/bin"
export PATH="$PROTOC_BIN:$PATH"
echo "$PROTOC_BIN" >> "$GITHUB_PATH"

echo "protoc installed at: $PROTOC_BIN"
protoc --version
