FROM rust:1.93-trixie

RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    weston \
    libwayland-dev \
    libxkbcommon-dev \
    libegl1 \
    libgles2 \
    libgbm1 \
    libdrm2 \
    libvulkan1 \
    mesa-vulkan-drivers \
    mesa-utils \
    wayland-utils \
    libudev-dev \
    libasound2-dev \
    libavutil-dev \
    libavcodec-dev \
    libavformat-dev \
    libavfilter-dev \
    libavdevice-dev \
    libswscale-dev \
    libswresample-dev \
    libdav1d-dev \
    fonts-dejavu-core \
    fonts-noto-core \
    clang \
    libclang-dev \
    pkg-config \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /work
