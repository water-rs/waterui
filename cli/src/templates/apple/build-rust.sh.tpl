#!/usr/bin/env bash
set -euo pipefail

# Skip Rust build when invoked by `water run` (it already builds the library)
if [ "${WATERUI_SKIP_RUST_BUILD:-}" = "1" ]; then
    echo "Skipping Rust build (managed by water run)"
    exit 0
fi

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/.." && pwd )"

# Xcode runs build scripts in a restricted shell environment without the user's
# full PATH. Add common tool locations explicitly.
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"

if ! command -v water >/dev/null 2>&1; then
    echo "error: the 'water' CLI is not on PATH. Install it with 'cargo install waterui-cli'." >&2
    echo "error: Searched in PATH: $PATH" >&2
    exit 1
fi

# Determine Rust target triple from Xcode environment variables
# PLATFORM_NAME: macosx, iphoneos, iphonesimulator, etc.
# ARCHS: arm64, x86_64, or "arm64 x86_64" for universal builds
PLATFORM_NAME="${PLATFORM_NAME:-macosx}"
ARCHS="${ARCHS:-arm64}"

# When Xcode passes multiple architectures (e.g., "arm64 x86_64"), pick the native one.
# For Apple Silicon Macs, prefer arm64; for Intel Macs, prefer x86_64.
if [[ "$ARCHS" == *" "* ]]; then
    NATIVE_ARCH=$(uname -m)
    if [[ "$ARCHS" == *"$NATIVE_ARCH"* ]]; then
        ARCHS="$NATIVE_ARCH"
    else
        # Fallback to first architecture in the list
        ARCHS="${ARCHS%% *}"
    fi
fi

# Map Xcode platform to water CLI platform name
case "$PLATFORM_NAME" in
    macosx)
        WATER_PLATFORM="macos"
        ;;
    iphoneos)
        WATER_PLATFORM="ios"
        ;;
    iphonesimulator)
        WATER_PLATFORM="ios-simulator"
        ;;
    *)
        echo "error: unsupported platform: $PLATFORM_NAME" >&2
        exit 1
        ;;
esac

CONFIGURATION_VALUE="${CONFIGURATION:-Debug}"
CLI_ARGS=(build --platform "$WATER_PLATFORM" --path "$PROJECT_ROOT")
if [ "$CONFIGURATION_VALUE" = "Release" ]; then
    CLI_ARGS+=(--release)
fi
if [ -n "${BUILT_PRODUCTS_DIR:-}" ]; then
    CLI_ARGS+=(--output-dir "$BUILT_PRODUCTS_DIR")
fi

exec water "${CLI_ARGS[@]}"
