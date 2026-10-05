# Benchmark environment for the Linux competitive benchmark
# (water-rs/waterui#1262). One image builds and runs every contestant under an
# identical display server (benchcomp, headless wlroots) and GPU driver
# (mesa 25: llvmpipe GL + lavapipe Vulkan). Wolfi/musl was suggested for
# lavapipe but Flutter and Electron prebuilts are glibc-only — a single glibc
# image keeps the display/driver uniform across contestants.
FROM debian:trixie

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
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
    nodejs npm python3 python3-venv \
    # misc runtime
    dbus-x11 procps \
    && rm -rf /var/lib/apt/lists/*

# rustup toolchain (pinned)
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:/opt/flutter/bin:/usr/local/bin:$PATH
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.98.1 --profile minimal \
    && rustc --version

# Flutter SDK (pinned)
RUN curl -sSf -o /tmp/flutter.tar.xz \
        https://storage.googleapis.com/flutter_infra_release/releases/stable/linux/flutter_linux_3.47.5-stable.tar.xz \
    && tar -C /opt -xf /tmp/flutter.tar.xz \
    && rm /tmp/flutter.tar.xz \
    && git config --global --add safe.directory /opt/flutter \
    && flutter --version

COPY docker/entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh
ENTRYPOINT ["/entrypoint.sh"]
CMD ["bash"]
