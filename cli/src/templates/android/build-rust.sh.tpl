#!/usr/bin/env bash
set -e

# This script builds the Rust library for all Android targets.

# Auto-detect the Android NDK path when ANDROID_NDK_HOME is not provided.
detect_ndk_home() {
    local -a search_roots=()
    local root

    add_root() {
        local path="$1"
        if [ -d "$path" ]; then
            search_roots+=("$path")
        fi
    }

    add_root "${ANDROID_NDK_HOME:-}"
    add_root "${ANDROID_SDK_ROOT:-}/ndk-bundle"
    add_root "${ANDROID_SDK_ROOT:-}/ndk"
    add_root "${ANDROID_HOME:-}/ndk-bundle"
    add_root "${ANDROID_HOME:-}/ndk"
    add_root "$HOME/Library/Android/sdk/ndk-bundle"
    add_root "$HOME/Library/Android/sdk/ndk"
    add_root "$HOME/Android/Sdk/ndk-bundle"
    add_root "$HOME/Android/Sdk/ndk"

    shopt -s nullglob
    for root in "${search_roots[@]}"; do
        if [ -f "$root/source.properties" ] || [ -d "$root/toolchains" ]; then
            printf '%s\n' "$root"
            shopt -u nullglob
            return 0
        fi

        local latest=""
        local candidate
        for candidate in "$root"/*; do
            [[ -d "$candidate" ]] || continue
            latest="$candidate"
        done
        if [ -n "$latest" ]; then
            printf '%s\n' "$latest"
            shopt -u nullglob
            return 0
        fi
    done
    shopt -u nullglob
    return 1
}

if [ -z "${ANDROID_NDK_HOME:-}" ]; then
    if detected_ndk=$(detect_ndk_home); then
        export ANDROID_NDK_HOME="$detected_ndk"
        echo "Using auto-detected ANDROID_NDK_HOME at $ANDROID_NDK_HOME"
    fi
fi

if [ -z "${ANDROID_NDK_HOME:-}" ]; then
    echo "Error: ANDROID_NDK_HOME is not set and could not be detected automatically."
    echo "Please install the Android NDK and set ANDROID_NDK_HOME to its path."
    exit 1
fi

detect_host_tag() {
    local -a candidates=()
    case "$(uname -s)" in
        Darwin)
            candidates=(darwin-arm64 darwin-aarch64 darwin-x86_64)
            ;;
        Linux)
            candidates=(linux-aarch64 linux-x86_64)
            ;;
        MINGW*|MSYS*|CYGWIN*|Windows_NT)
            candidates=(windows-x86_64)
            ;;
        *)
            candidates=()
            ;;
    esac

    local tag
    for tag in "${candidates[@]}"; do
        if [ -d "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$tag/bin" ]; then
            printf '%s\n' "$tag"
            return 0
        fi
    done

    for tag in "$ANDROID_NDK_HOME"/toolchains/llvm/prebuilt/*; do
        if [ -d "$tag/bin" ]; then
            printf '%s\n' "$(basename "$tag")"
            return 0
        fi
    done

    return 1
}

if host_tag=$(detect_host_tag); then
    toolchain_bin="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$host_tag/bin"
    export PATH="$toolchain_bin:$PATH"
else
    echo "Warning: Unable to determine NDK toolchain host tag; compiler binaries may be missing from PATH."
fi

# Install the Rust targets if needed:
# rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android

CRATE_NAME=__CRATE_NAME__

# Build for all Android targets
cargo build --target aarch64-linux-android --release
cargo build --target armv7-linux-androideabi --release
cargo build --target i686-linux-android --release
cargo build --target x86_64-linux-android --release

# Copy the libraries to the jniLibs directory
JNI_DIR="android/app/src/main/jniLibs"

mkdir -p "$JNI_DIR/arm64-v8a"
cp "target/aarch64-linux-android/release/lib${CRATE_NAME}.so" "$JNI_DIR/arm64-v8a/"

mkdir -p "$JNI_DIR/armeabi-v7a"
cp "target/armv7-linux-androideabi/release/lib${CRATE_NAME}.so" "$JNI_DIR/armeabi-v7a/"

mkdir -p "$JNI_DIR/x86"
cp "target/i686-linux-android/release/lib${CRATE_NAME}.so" "$JNI_DIR/x86/"

mkdir -p "$JNI_DIR/x86_64"
cp "target/x86_64-linux-android/release/lib${CRATE_NAME}.so" "$JNI_DIR/x86_64/"

echo "Rust libraries copied to $JNI_DIR"
