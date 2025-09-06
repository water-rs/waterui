#!/bin/bash

# Exit on error
set -e

# Debug output
echo "Starting Rust build script..."
echo "Current directory: $(pwd)"
echo "SRCROOT: $SRCROOT"
echo "BUILT_PRODUCTS_DIR: $BUILT_PRODUCTS_DIR"

# Get the directory of this script
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$SCRIPT_DIR/../.."

# Configuration
RUST_TARGET_DIR="$PROJECT_ROOT/target"
CARGO_MANIFEST="$PROJECT_ROOT/demo/Cargo.toml"

# Determine target architecture based on Xcode build settings
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
else
    # Default to macOS native architecture
    if [ "$(uname -m)" = "x86_64" ]; then
        RUST_TARGET="x86_64-apple-darwin"
    else
        RUST_TARGET="aarch64-apple-darwin"
    fi
fi

# Determine build profile
if [ "$CONFIGURATION" = "Release" ]; then
    CARGO_PROFILE="release"
    CARGO_ARGS="--release"
    PROFILE_DIR="release"
else
    CARGO_PROFILE="debug"
    CARGO_ARGS=""
    PROFILE_DIR="debug"
fi

echo "Building Rust library for target: $RUST_TARGET"
echo "Configuration: $CONFIGURATION"
echo "Cargo profile: $CARGO_PROFILE"

# Ensure cargo is in PATH (Xcode doesn't inherit user's PATH)
export PATH="$HOME/.cargo/bin:$PATH"

# Ensure cargo is available
if ! command -v cargo &> /dev/null; then
    echo "Error: cargo not found. Please install Rust."
    echo "Visit https://rustup.rs to install Rust"
    exit 1
fi

# Build the Rust library
cd "$PROJECT_ROOT/demo"

# Install target if needed
rustup target add "$RUST_TARGET" 2>/dev/null || true

# Build for the specific target
cargo build --target "$RUST_TARGET" $CARGO_ARGS

# Create output directory if it doesn't exist
OUTPUT_DIR="$BUILT_PRODUCTS_DIR"
if [ -z "$OUTPUT_DIR" ]; then
    OUTPUT_DIR="$SCRIPT_DIR/build"
fi
mkdir -p "$OUTPUT_DIR"

# Path to the built library
RUST_LIB="$RUST_TARGET_DIR/$RUST_TARGET/$PROFILE_DIR/libdemo.a"

# Verify the library was built
if [ ! -f "$RUST_LIB" ]; then
    echo "Error: Rust library not found at $RUST_LIB"
    exit 1
fi

# Copy library to build products directory for easy access
cp "$RUST_LIB" "$OUTPUT_DIR/libdemo.a"

echo "Successfully built Rust library at: $RUST_LIB"
echo "Copied to: $OUTPUT_DIR/libdemo.a"

# Export the library path for Xcode to use
echo "RUST_LIBRARY_PATH=$RUST_LIB" > "$SCRIPT_DIR/rust_build_info.xcconfig"