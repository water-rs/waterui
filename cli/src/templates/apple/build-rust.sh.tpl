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
# ARCHS: arm64, x86_64, etc.
PLATFORM_NAME="${PLATFORM_NAME:-macosx}"
ARCHS="${ARCHS:-arm64}"

# Map Xcode platform/arch to Rust target triple
case "$PLATFORM_NAME" in
    macosx)
        case "$ARCHS" in
            arm64) TARGET="aarch64-apple-darwin" ;;
            x86_64) TARGET="x86_64-apple-darwin" ;;
            *) echo "error: unsupported macOS architecture: $ARCHS" >&2; exit 1 ;;
        esac
        ;;
    iphoneos)
        case "$ARCHS" in
            arm64) TARGET="aarch64-apple-ios" ;;
            *) echo "error: unsupported iOS architecture: $ARCHS" >&2; exit 1 ;;
        esac
        ;;
    iphonesimulator)
        case "$ARCHS" in
            arm64) TARGET="aarch64-apple-ios-sim" ;;
            x86_64) TARGET="x86_64-apple-ios" ;;
            *) echo "error: unsupported iOS simulator architecture: $ARCHS" >&2; exit 1 ;;
        esac
        ;;
    *)
        echo "error: unsupported platform: $PLATFORM_NAME" >&2
        exit 1
        ;;
esac

CONFIGURATION_VALUE="${CONFIGURATION:-Debug}"
CLI_ARGS=(build "$TARGET" --project "$PROJECT_ROOT")
if [ "$CONFIGURATION_VALUE" = "Release" ]; then
    CLI_ARGS+=(--release)
fi
if [ -n "${BUILT_PRODUCTS_DIR:-}" ]; then
    CLI_ARGS+=(--output-dir "$BUILT_PRODUCTS_DIR")
fi

exec water "${CLI_ARGS[@]}"
