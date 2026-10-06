# Benchmark environment for the Linux competitive benchmark
# (water-rs/waterui#1262). One image builds and runs every contestant under an
# identical display server (benchcomp, headless wlroots) and GPU driver
# (mesa: llvmpipe GL + lavapipe Vulkan). Wolfi/musl was suggested for
# lavapipe but Flutter and Electron prebuilts are glibc-only — a single glibc
# image keeps the display/driver uniform across contestants.
#
# Every input is pinned: the base image by index digest, apt packages by a
# dated snapshot.debian.org archive, rustup-init by sha256, the Rust
# toolchain by the repository's own rust-toolchain.toml (passed as
# RUST_CHANNEL build-arg), and the Flutter and Node tarballs by version and
# sha256 from ../manifest.toml, passed as build args by the runner — this
# file declares no default for any of them, so an image cannot be built
# from a pin the manifest does not hold.
FROM debian:trixie@sha256:9cc080028c43b27d2074d63a5f9caf7166d731494965616c1a6d2827a004585c

ENV DEBIAN_FRONTEND=noninteractive

# Dated snapshot archive: every package resolved is the one published at
# 2026-09-25T00:00Z on any host — no floating deb.debian.org metadata.
# The date must be at or after the base image's package set (trixie 13.7);
# an earlier snapshot resolves -dev packages older than the installed
# runtime libs (uuid-dev 2.41-5 vs libuuid1 2.41.5-0+deb13u1) and the
# install fails.
RUN echo 'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/20260925T000000Z trixie main' \
        > /etc/apt/sources.list \
    && echo 'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/20260925T000000Z trixie-updates main' \
        >> /etc/apt/sources.list \
    && echo 'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian-security/20260925T000000Z trixie-security main' \
        >> /etc/apt/sources.list \
    && rm -f /etc/apt/sources.list.d/debian.sources \
    && apt-get update && apt-get install -y --no-install-recommends \
    # benchcomp + measurement stack
    libwlroots-0.18-dev wayland-protocols libwayland-bin libdrm-dev libgbm-dev \
    libxkbcommon-dev libpixman-1-dev libinput-dev \
    # GPU stack (software): llvmpipe GL + lavapipe Vulkan + EGL
    mesa-vulkan-drivers libvulkan1 libegl-mesa0 libgles2 libgl1-mesa-dri libgbm1 \
    # vulkaninfo (--summary) is the maintained Vulkan enumeration the
    # adapter probe parses — never a hand-written ctypes Vulkan ABI
    vulkan-tools \
    # wayland + x11 client libs
    libwayland-client0 libwayland-egl1 libx11-6 libxcb1 \
    # fonts: Latin + CJK + emoji coverage for W4
    fontconfig fonts-dejavu-core fonts-noto-cjk fonts-noto-color-emoji \
    # GTK4 contestant build+run
    libgtk-4-dev \
    # GTK3 (Flutter shell) + Electron runtime deps
    libgtk-3-dev libnss3 libnspr4 libasound2t64 libxrandr2 libpangocairo-1.0-0 \
    libatk1.0-0t64 libatk-bridge2.0-0t64 libcups2t64 libxcomposite1 libxdamage1 \
    libxfixes3 libxss1 \
    # toolchain
    build-essential gcc g++ clang cmake ninja-build pkg-config \
    curl xz-utils unzip git ca-certificates \
    python3 python3-venv \
    # misc runtime
    dbus-x11 procps \
    && rm -rf /var/lib/apt/lists/*

# rustup toolchain — installer sha256-verified against the immutable
# archive URL (dist/ floats with each rustup release). RUST_CHANNEL is
# the EXACT version the runner resolved from rust-toolchain.toml at
# image build (e.g. 1.90.0 or nightly-2025-09-14) — never a floating
# channel name; the installed `rustc --version` is recorded into
# results.versions.
ARG TARGETARCH
ARG RUST_CHANNEL
ARG RUSTUP_VERSION=1.29.1
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:/opt/flutter/bin:/opt/node/bin:/usr/local/bin:$PATH
RUN set -eu; \
    case "$TARGETARCH" in \
        amd64) RUSTUP_ARCH=x86_64-unknown-linux-gnu; \
               RUSTUP_SHA256=dda7234360b7f578ca8b0ddcb80145646fa61a67c1720a5abc7051b35c9fcb71 ;; \
        arm64) RUSTUP_ARCH=aarch64-unknown-linux-gnu; \
               RUSTUP_SHA256=15f6e4ce9f583b929c996c91562bad6d4454f3281de858b02cdfdef615fac433 ;; \
        *) echo "unsupported image arch: $TARGETARCH" >&2; exit 1 ;; \
    esac; \
    curl -sSf -o /tmp/rustup-init \
        "https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/${RUSTUP_ARCH}/rustup-init"; \
    echo "${RUSTUP_SHA256}  /tmp/rustup-init" | sha256sum -c -; \
    chmod +x /tmp/rustup-init; \
    /tmp/rustup-init -y --default-toolchain "$RUST_CHANNEL" --profile minimal; \
    rm /tmp/rustup-init; \
    # the checkout's rust-toolchain.toml asks for clippy+rustfmt — missing
    # components make the cargo shim sync the channel into /opt/rustup
    rustup component add clippy rustfmt; \
    # containers run as the host uid — the rustup home must be writable or
    # the cargo shim's channel check fails on /opt/rustup/tmp
    chmod -R a+rwX /opt/rustup /opt/cargo; \
    rustc --version; \
    cargo clippy --version

# Flutter SDK — tarball sha256-verified against the pinned version
# (manifest [contestants.flutter] flutter_version / flutter_sha256)
ARG FLUTTER_VERSION
ARG FLUTTER_SHA256
RUN : "${FLUTTER_VERSION:?build arg from manifest [contestants.flutter]}" \
    && : "${FLUTTER_SHA256:?build arg from manifest [contestants.flutter]}" \
    && curl -sSf -o /tmp/flutter.tar.xz \
        "https://storage.googleapis.com/flutter_infra_release/releases/stable/linux/flutter_linux_${FLUTTER_VERSION}-stable.tar.xz" \
    && echo "${FLUTTER_SHA256}  /tmp/flutter.tar.xz" | sha256sum -c - \
    && tar -C /opt -xf /tmp/flutter.tar.xz \
    && rm /tmp/flutter.tar.xz \
    && git config --global --add safe.directory /opt/flutter \
    && flutter --version

# Node.js — the official tarball for the pinned version, sha256-verified
# (manifest [toolchain.node]: version and the nodejs.org SHASUMS256.txt
# value per architecture). It only drives `npm ci` for the Electron
# contestant, whose toolchain requires node >= 22.12; trixie's packaged
# nodejs is 20.x.
ARG NODE_VERSION
ARG NODE_SHA256_AMD64
ARG NODE_SHA256_ARM64
RUN set -eu; \
    : "${NODE_VERSION:?build arg from manifest [toolchain.node]}"; \
    case "$TARGETARCH" in \
        amd64) NODE_ARCH=x64; \
               NODE_SHA256="${NODE_SHA256_AMD64:?build arg from manifest [toolchain.node]}" ;; \
        arm64) NODE_ARCH=arm64; \
               NODE_SHA256="${NODE_SHA256_ARM64:?build arg from manifest [toolchain.node]}" ;; \
        *) echo "unsupported image arch: $TARGETARCH" >&2; exit 1 ;; \
    esac; \
    curl -sSf -o /tmp/node.tar.xz \
        "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-${NODE_ARCH}.tar.xz"; \
    echo "${NODE_SHA256}  /tmp/node.tar.xz" | sha256sum -c -; \
    mkdir /opt/node; \
    tar -C /opt/node --strip-components=1 -xf /tmp/node.tar.xz; \
    rm /tmp/node.tar.xz; \
    test "$(node --version)" = "v${NODE_VERSION}"

COPY docker/entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh
ENTRYPOINT ["/entrypoint.sh"]
CMD ["bash"]
