#!/usr/bin/env bash

# Fetches, builds and installs WPE WebKit into <work-directory>/runtime —
# the multi-hour stage of build-runtime.sh, kept as its own entry point so the
# CI workflow can wrap exactly this stage in an actions/cache pair. Everything
# this script does is idempotent: an extraction marker guards the tarball work,
# and a completion marker written into the install prefix after a successful
# `cmake --install` guards the build itself. Because the marker lives inside
# the cached prefix, a cache-restored prefix skips straight past the compile.
#
# The dependency install runs on every invocation, marker or not: later
# stages vendor the host's runtime libraries into the packaged artifact, and
# the WebKit source tree is needed for the license bundle — neither is part
# of the cached prefix.

set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "usage: $0 <work-directory>" >&2
    exit 2
fi

repo_root="$(git rev-parse --show-toplevel)"
source_configuration="$repo_root/components/platform/browser-wpe/runtime/source.toml"
work_directory="$(mkdir -p "$1" && cd "$1" && pwd)"

configuration_value() {
    local key="$1"
    sed -n "s/^${key} = \"\\([^\"]*\\)\"$/\\1/p" "$source_configuration"
}

version="$(configuration_value version)"
source_url="$(configuration_value source_url)"
source_sha256="$(configuration_value source_sha256)"
glib_dependencies_url="$(configuration_value glib_dependencies_url)"
glib_dependencies_sha256="$(configuration_value glib_dependencies_sha256)"
minimum_gcc="$(configuration_value minimum_gcc)"

if [[ -z "$version" || -z "$source_url" || -z "$source_sha256" || -z "$glib_dependencies_url" || -z "$glib_dependencies_sha256" || -z "$minimum_gcc" ]]; then
    echo "invalid WPE runtime source configuration" >&2
    exit 1
fi

c_compiler="${CC:-cc}"
cxx_compiler="${CXX:-c++}"
command -v "$c_compiler" >/dev/null || {
    echo "WPE runtime C compiler is unavailable: $c_compiler" >&2
    exit 1
}
command -v "$cxx_compiler" >/dev/null || {
    echo "WPE runtime C++ compiler is unavailable: $cxx_compiler" >&2
    exit 1
}
compiler_banner="$($c_compiler --version | head -n 1)"
if [[ "$compiler_banner" != *gcc* && "$compiler_banner" != *GCC* ]]; then
    echo "WPE runtime requires GCC $minimum_gcc or newer; $c_compiler is $compiler_banner" >&2
    exit 1
fi
actual_gcc="$($c_compiler -dumpfullversion -dumpversion)"
oldest_gcc="$(printf '%s\n%s\n' "$actual_gcc" "$minimum_gcc" | sort -V | head -n 1)"
if [[ "$oldest_gcc" != "$minimum_gcc" ]]; then
    echo "WPE WebKit $version requires GCC $minimum_gcc or newer; $c_compiler reports $actual_gcc" >&2
    exit 1
fi

archive="$work_directory/wpewebkit.tar.xz"
source_directory="$work_directory/wpewebkit-$version"
build_directory="$work_directory/build"
prefix="$work_directory/runtime"

# The marker — not the directory — records a completed extraction, so a run
# interrupted mid-tar fetches and extracts again instead of trusting a
# partial tree.
extracted_marker="$work_directory/.extracted-$version"
if [[ ! -f "$extracted_marker" ]]; then
    curl --fail --location --retry 3 --output "$archive" "$source_url"
    printf '%s  %s\n' "$source_sha256" "$archive" | sha256sum --check
    rm -rf "$source_directory"
    tar -xJf "$archive" -C "$work_directory"
    touch "$extracted_marker"
fi

# `Tools/wpe/dependencies/apt` sources this file and the release tarball does
# not ship it, so upstream's own installer exits before installing anything.
# Restore it from the tag the tarball was cut from rather than keeping a second,
# hand-copied dependency list in this repository.
glib_dependencies="$source_directory/Tools/glib/dependencies/apt"
if [[ ! -f "$glib_dependencies" ]]; then
    mkdir -p "$(dirname "$glib_dependencies")"
    curl --fail --location --retry 3 --output "$glib_dependencies" "$glib_dependencies_url"
    printf '%s  %s\n' "$glib_dependencies_sha256" "$glib_dependencies" | sha256sum --check
    chmod +x "$glib_dependencies"
fi

# Upstream's list mixes developer tooling in with the build dependencies, and
# two entries make the whole apt transaction unsatisfiable on a current CI
# image:
#   * `git-svn` pins `git (< 1:2.34.1-.)`, while GitHub's runner images install
#     git from a PPA — 2.55 against 22.04's 2.34. It is tooling for the SVN
#     workflow WebKit has long since left behind; nothing in this build reads
#     it. apt cannot install it, and one unusable package aborts everything
#     else with it.
#   * `libgstreamer1.0-dev` needs `libunwind-dev`, which apt does not pull in
#     on its own here.
# Drop the first from the list and install the second alongside our own
# additions, so the installer fails only for reasons that actually matter.
sed -i '/git-svn/d' \
    "$glib_dependencies" \
    "$source_directory/Tools/wpe/dependencies/apt"

# Before upstream's installer, not after: it is `libgstreamer1.0-dev` inside
# that installer's own list that needs this, so satisfying it afterwards is too
# late — the installer has already aborted the transaction.
sudo apt-get install -y --no-install-recommends libunwind-dev

sudo "$source_directory/Tools/wpe/install-dependencies"
sudo apt-get install -y --no-install-recommends \
    bubblewrap \
    cmake \
    gstreamer1.0-plugins-base \
    gstreamer1.0-plugins-good \
    ninja-build \
    patchelf \
    pax-utils \
    xdg-dbus-proxy

# `USE_LIBBACKTRACE` defaults on and is a hard requirement when it is, but no
# Debian or Ubuntu release packages libbacktrace, so configuring fails on every
# apt-based host. It only symbolizes WebKit's own crash logs, which a shipped
# runtime does not print.
# `USE_JPEGXL` defaults on the same way, but jammy has no `libjxl-dev` — the
# package entered Ubuntu at 23.04 — so the flag fails the one image the
# artifact's glibc floor allows. JPEG XL decoding is optional for the embedded
# runtime.
# `ENABLE_WPE_PLATFORM_DRM` defaults on too, and WPEPlatform's DRM display
# calls `drmModeCreateDumbBuffer`, which entered libdrm at 2.4.114 — jammy
# ships 2.4.113. The embedded runtime only ever creates the headless display
# (`wpe_display_headless_new`), so the DRM display is dead code here.
# The completion marker — not the files cmake installed — records a finished
# build, so a run interrupted mid-install rebuilds instead of trusting a
# partial prefix, and a cache-restored prefix skips the compile entirely.
installed_marker="$prefix/.webkit-installed-$version"
if [[ ! -f "$installed_marker" ]]; then
    cmake \
        -S "$source_directory" \
        -B "$build_directory" \
        -G Ninja \
        -DPORT=WPE \
        -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_INSTALL_PREFIX="$prefix" \
        -DCMAKE_INSTALL_LIBDIR=lib \
        -DCMAKE_INSTALL_LIBEXECDIR=libexec \
        -DCMAKE_C_COMPILER="$c_compiler" \
        -DCMAKE_CXX_COMPILER="$cxx_compiler" \
        -DBWRAP_EXECUTABLE=/usr/bin/bwrap \
        -DDBUS_PROXY_EXECUTABLE=/usr/bin/xdg-dbus-proxy \
        -DENABLE_API_TESTS=OFF \
        -DENABLE_BUBBLEWRAP_SANDBOX=ON \
        -DENABLE_DOCUMENTATION=OFF \
        -DENABLE_INTROSPECTION=OFF \
        -DENABLE_JOURNALD_LOG=OFF \
        -DENABLE_LAYOUT_TESTS=OFF \
        -DENABLE_MINIBROWSER=OFF \
        -DENABLE_WPE_LEGACY_API=OFF \
        -DENABLE_WPE_PLATFORM=ON \
        -DENABLE_WPE_PLATFORM_DRM=OFF \
        -DUSE_JPEGXL=OFF \
        -DUSE_LIBBACKTRACE=OFF
    cmake --build "$build_directory" --parallel "${WATERUI_WPE_BUILD_JOBS:-$(nproc)}"
    cmake --install "$build_directory"
    touch "$installed_marker"
fi
