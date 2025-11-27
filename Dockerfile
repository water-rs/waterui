# WaterUI Builder Image
# Build environment for WaterUI apps with Android and WebAssembly support
#
# Usage:
#   docker run --rm -v $(pwd):/workspace ghcr.io/water-rs/water-builder water build android
#   docker run --rm -v $(pwd):/workspace ghcr.io/water-rs/water-builder water run --platform web
#
# This image includes:
#   - Rust stable toolchain with Android and WASM targets
#   - Android SDK (platform-tools, build-tools, NDK)
#   - Java 21 (for Gradle)
#   - wasm-pack for web builds
#   - water CLI pre-installed
#   - sccache with pre-warmed compilation cache

FROM ubuntu:24.04

LABEL org.opencontainers.image.source="https://github.com/water-rs/waterui"
LABEL org.opencontainers.image.description="WaterUI app builder with Rust, Android SDK/NDK, and WebAssembly toolchains"

# Avoid interactive prompts
ENV DEBIAN_FRONTEND=noninteractive

# Android SDK/NDK versions
ENV ANDROID_SDK_ROOT=/opt/android-sdk
ENV ANDROID_HOME=/opt/android-sdk
ENV ANDROID_NDK_VERSION=27.2.12479018
ENV ANDROID_NDK_HOME=${ANDROID_SDK_ROOT}/ndk/${ANDROID_NDK_VERSION}
ENV ANDROID_BUILD_TOOLS_VERSION=35.0.0
ENV ANDROID_PLATFORM_VERSION=35

# Java version
ENV JAVA_HOME=/usr/lib/jvm/java-21-openjdk-amd64

# Rust paths
ENV RUSTUP_HOME=/opt/rustup
ENV CARGO_HOME=/opt/cargo
ENV PATH="${CARGO_HOME}/bin:${ANDROID_SDK_ROOT}/cmdline-tools/latest/bin:${ANDROID_SDK_ROOT}/platform-tools:${PATH}"

# sccache configuration
ENV SCCACHE_DIR=/opt/sccache-cache
ENV SCCACHE_CACHE_SIZE=10G
ENV RUSTC_WRAPPER=sccache

# Install system dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    # Build essentials
    build-essential \
    pkg-config \
    # SSL and crypto
    libssl-dev \
    ca-certificates \
    # Git and utilities
    git \
    curl \
    wget \
    unzip \
    # Java (required for Android/Gradle)
    openjdk-21-jdk-headless \
    && rm -rf /var/lib/apt/lists/*

# Install Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --no-modify-path \
    && chmod -R a+rw ${RUSTUP_HOME} ${CARGO_HOME}

# Add Android and WASM targets
RUN rustup target add \
    aarch64-linux-android \
    armv7-linux-androideabi \
    x86_64-linux-android \
    i686-linux-android \
    wasm32-unknown-unknown

# Install sccache, wasm-pack, and water CLI
RUN cargo install sccache --locked \
    && cargo install wasm-pack --locked \
    && cargo install waterui-cli --locked

# Install Android SDK command-line tools
RUN mkdir -p ${ANDROID_SDK_ROOT}/cmdline-tools \
    && cd ${ANDROID_SDK_ROOT}/cmdline-tools \
    && wget -q https://dl.google.com/android/repository/commandlinetools-linux-11076708_latest.zip -O cmdline-tools.zip \
    && unzip -q cmdline-tools.zip \
    && rm cmdline-tools.zip \
    && mv cmdline-tools latest

# Accept Android SDK licenses and install components
RUN yes | sdkmanager --licenses > /dev/null 2>&1 || true \
    && sdkmanager --update \
    && sdkmanager \
    "platform-tools" \
    "platforms;android-${ANDROID_PLATFORM_VERSION}" \
    "build-tools;${ANDROID_BUILD_TOOLS_VERSION}" \
    "ndk;${ANDROID_NDK_VERSION}"

# Configure cargo linkers for Android NDK
RUN mkdir -p ${CARGO_HOME} && \
    printf '%s\n' \
    '[target.aarch64-linux-android]' \
    "linker = \"${ANDROID_NDK_HOME}/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android24-clang\"" \
    '' \
    '[target.armv7-linux-androideabi]' \
    "linker = \"${ANDROID_NDK_HOME}/toolchains/llvm/prebuilt/linux-x86_64/bin/armv7a-linux-androideabi24-clang\"" \
    '' \
    '[target.x86_64-linux-android]' \
    "linker = \"${ANDROID_NDK_HOME}/toolchains/llvm/prebuilt/linux-x86_64/bin/x86_64-linux-android24-clang\"" \
    '' \
    '[target.i686-linux-android]' \
    "linker = \"${ANDROID_NDK_HOME}/toolchains/llvm/prebuilt/linux-x86_64/bin/i686-linux-android24-clang\"" \
    >> ${CARGO_HOME}/config.toml

# Pre-warm sccache by compiling waterui and waterui-ffi for all targets
# This ensures users get fast builds from the start
RUN mkdir -p ${SCCACHE_DIR} && chmod -R a+rw ${SCCACHE_DIR} \
    && mkdir -p /tmp/waterui-warmup && cd /tmp/waterui-warmup \
    && cargo new --lib warmup && cd warmup \
    && printf '[dependencies]\nwaterui = "*"\nwaterui-ffi = "*"\n' >> Cargo.toml \
    # Build for Android targets
    && cargo build --release --target aarch64-linux-android 2>/dev/null || true \
    # Build for WASM
    && cargo build --release --target wasm32-unknown-unknown 2>/dev/null || true \
    # Cleanup build artifacts but keep sccache
    && cd / && rm -rf /tmp/waterui-warmup \
    && sccache --show-stats

# Create non-root user
RUN useradd -m -u 1000 -s /bin/bash water \
    && mkdir -p /workspace \
    && chown -R water:water /workspace ${RUSTUP_HOME} ${CARGO_HOME} ${SCCACHE_DIR}

USER water
WORKDIR /workspace

# Verify installations
RUN rustc --version && cargo --version && java --version && water --version && sccache --version

ENTRYPOINT ["water"]
CMD ["--help"]
