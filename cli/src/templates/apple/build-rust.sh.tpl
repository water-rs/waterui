#!/bin/bash
set -e

SCRIPT_DIR="$( cd "$( dirname "${{BASH_SOURCE[0]}}" )" && pwd )"
PROJECT_ROOT="$( cd "${{SCRIPT_DIR}}/.." && pwd )"

LIB_NAME="__LIB_NAME__"
TARGET_DIR="$PROJECT_ROOT/target"

if [ -z "$CONFIGURATION" ]; then
    CONFIGURATION="Debug"
fi

if [ "$PLATFORM_NAME" = "iphonesimulator" ]; then
    if [ "$ARCHS" = "x86_64" ]; then
        RUST_TARGET="x86_64-apple-ios"
    else
        RUST_TARGET="aarch64-apple-ios-sim"
    fi
elif [ "$PLATFORM_NAME" = "iphoneos" ]; then
    RUST_TARGET="aarch64-apple-ios"
elif [ "$PLATFORM_NAME" = "macosx" ]; then
    if [ "$ARCHS" = "x86_64" ]; then
        RUST_TARGET="x86_64-apple-darwin"
    else
        RUST_TARGET="aarch64-apple-darwin"
    fi
elif [ "$PLATFORM_NAME" = "xrsimulator" ]; then
    RUST_TARGET="aarch64-apple-ios-sim"
elif [ "$PLATFORM_NAME" = "xros" ]; then
    RUST_TARGET="aarch64-apple-ios"
else
    if [ "$(uname -m)" = "x86_64" ]; then
        RUST_TARGET="x86_64-apple-darwin"
    else
        RUST_TARGET="aarch64-apple-darwin"
    fi
fi

if [ "$CONFIGURATION" = "Release" ]; then
    PROFILE="release"
    CARGO_ARGS="--release"
else
    PROFILE="debug"
    CARGO_ARGS=""
fi

export PATH="$HOME/.cargo/bin:$PATH"

if ! command -v cargo >/dev/null 2>&1; then
    echo "error: cargo not found. Install Rust from https://rustup.rs"
    exit 1
fi

rustup target add "$RUST_TARGET" >/dev/null 2>&1 || true

cd "$PROJECT_ROOT"
cargo build --target "$RUST_TARGET" $CARGO_ARGS

OUTPUT_DIR="$BUILT_PRODUCTS_DIR"
if [ -z "$OUTPUT_DIR" ]; then
    OUTPUT_DIR="$SCRIPT_DIR/build"
fi
mkdir -p "$OUTPUT_DIR"

RUST_LIB="$TARGET_DIR/$RUST_TARGET/$PROFILE/lib__LIB_NAME__.a"
if [ ! -f "$RUST_LIB" ]; then
    echo "error: Rust library not found at $RUST_LIB"
    exit 1
fi

cp "$RUST_LIB" "$OUTPUT_DIR/lib__LIB_NAME__.a"
echo "RUST_LIBRARY_PATH=$RUST_LIB" > "$SCRIPT_DIR/rust_build_info.xcconfig"
