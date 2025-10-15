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

configure_toolchain_env() {
    local triple="$1"
    local bin_prefix="$2"
    local api="$3"

    local clang="$toolchain_bin/${bin_prefix}${api}-clang"
    if [ ! -x "$clang" ]; then
        clang="$toolchain_bin/${bin_prefix}-clang"
    fi
    if [ ! -x "$clang" ]; then
        echo "Warning: clang for $triple not found in $toolchain_bin"
        return
    fi

    local clang_pp="${clang}++"
    if [ ! -x "$clang_pp" ]; then
        clang_pp="$toolchain_bin/${bin_prefix}${api}-clang++"
    fi
    if [ ! -x "$clang_pp" ]; then
        clang_pp="$toolchain_bin/${bin_prefix}-clang++"
    fi

    local env_triple="${triple//-/_}"
    local upper_triple
    upper_triple=$(printf '%s' "$env_triple" | tr '[:lower:]' '[:upper:]')

    local cc_var="CC_${env_triple}"
    local cxx_var="CXX_${env_triple}"
    local linker_var="CARGO_TARGET_${upper_triple}_LINKER"

    printf -v "$cc_var" '%s' "$clang"
    printf -v "$cxx_var" '%s' "$clang_pp"
    printf -v "$linker_var" '%s' "$clang"
    export "$cc_var" "$cxx_var" "$linker_var"

    local ar_path="$toolchain_bin/llvm-ar"
    if [ -x "$ar_path" ]; then
        local ar_var="AR_${env_triple}"
        local cargo_ar_var="CARGO_TARGET_${upper_triple}_AR"
        printf -v "$ar_var" '%s' "$ar_path"
        printf -v "$cargo_ar_var" '%s' "$ar_path"
        export "$ar_var" "$cargo_ar_var"
    fi

    local ranlib_path="$toolchain_bin/llvm-ranlib"
    if [ -x "$ranlib_path" ]; then
        local ranlib_var="RANLIB_${env_triple}"
        printf -v "$ranlib_var" '%s' "$ranlib_path"
        export "$ranlib_var"
    fi

    return 0
}

ANDROID_TARGET_SPECS=(
    "aarch64-linux-android:arm64-v8a:21:aarch64-linux-android"
    "x86_64-linux-android:x86_64:21:x86_64-linux-android"
    "armv7-linux-androideabi:armeabi-v7a:19:armv7a-linux-androideabi"
    "i686-linux-android:x86:19:i686-linux-android"
)

if command -v rustup >/dev/null 2>&1; then
    installed_targets="$(rustup target list --installed 2>/dev/null || true)"
else
    installed_targets=""
fi

target_installed() {
    local triple="$1"
    if [ -z "$installed_targets" ]; then
        return 0
    fi
    printf '%s\n' "$installed_targets" | grep -qx "$triple"
}

parse_requested_targets() {
    local raw="${ANDROID_BUILD_TARGETS:-}"
    raw="${raw//,/ }"
    read -r -a requested_targets <<< "$raw"
}

declare -a requested_targets=()
if [ -n "${ANDROID_BUILD_TARGETS:-}" ]; then
    parse_requested_targets
else
    for spec in "${ANDROID_TARGET_SPECS[@]}"; do
        IFS=: read -r triple _ <<< "$spec"
        if target_installed "$triple"; then
            requested_targets+=("$triple")
        fi
    done

    if [ ${#requested_targets[@]} -eq 0 ]; then
        requested_targets=("aarch64-linux-android")
    fi
fi

contains_target() {
    local needle="$1"
    shift || true
    for candidate in "$@"; do
        if [ "$candidate" = "$needle" ]; then
            return 0
        fi
    done
    return 1
}

declare -a selected_specs=()
for spec in "${ANDROID_TARGET_SPECS[@]}"; do
    IFS=: read -r triple _ <<< "$spec"
    if contains_target "$triple" "${requested_targets[@]}"; then
        if target_installed "$triple"; then
            selected_specs+=("$spec")
        else
            echo "Skipping $triple (rustup target not installed)."
        fi
    fi
done

if [ ${#selected_specs[@]} -eq 0 ]; then
    echo "Error: no Android Rust targets are available to build."
    echo "Install the desired targets with 'rustup target add aarch64-linux-android'."
    exit 1
fi

echo "Building Rust targets: ${requested_targets[*]}"

declare -a built_specs=()

# Install the Rust targets if needed:
# rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android

CRATE_NAME=__CRATE_NAME__
CRATE_FILE_BASENAME="${CRATE_NAME//-/_}"

# Build for selected Android targets
for spec in "${selected_specs[@]}"; do
    IFS=: read -r triple abi api bin_prefix <<< "$spec"
    echo "Configuring toolchain for $triple..."
    if ! configure_toolchain_env "$triple" "$bin_prefix" "$api"; then
        echo "Skipping $triple (toolchain binaries missing)."
        continue
    fi
    echo "Building target $triple..."
    cargo build --target "$triple" --release --package "$CRATE_NAME"
    built_specs+=("$spec")
done

if [ ${#built_specs[@]} -eq 0 ]; then
    echo "Error: failed to build any Android targets."
    exit 1
fi

# Copy the libraries to the jniLibs directory
JNI_DIR="android/app/src/main/jniLibs"

for spec in "${built_specs[@]}"; do
    IFS=: read -r triple abi _ _ <<< "$spec"
    mkdir -p "$JNI_DIR/$abi"
    cp "target/$triple/release/lib${CRATE_FILE_BASENAME}.so" "$JNI_DIR/$abi/"
done

echo "Rust libraries copied to $JNI_DIR"

echo "Copying libc++_shared.so..."
for spec in "${built_specs[@]}"; do
    IFS=: read -r triple abi _ _ <<< "$spec"

    libcxx_src="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$host_tag/sysroot/usr/lib/$triple/libc++_shared.so"
    libcxx_dst="$JNI_DIR/$abi/libc++_shared.so"

    if [ -f "$libcxx_src" ]; then
        mkdir -p "$JNI_DIR/$abi"
        cp "$libcxx_src" "$libcxx_dst"
        echo "  → Copied libc++_shared.so for $abi"
    else
        echo "  ⚠️  libc++_shared.so not found for $abi ($libcxx_src)"
    fi
done