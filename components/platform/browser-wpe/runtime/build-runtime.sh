#!/usr/bin/env bash

set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "usage: $0 <output-directory>" >&2
    exit 2
fi

repo_root="$(git rev-parse --show-toplevel)"
runtime_directory="$repo_root/components/platform/browser-wpe/runtime"
source_configuration="$runtime_directory/source.toml"
output_directory="$(mkdir -p "$1" && cd "$1" && pwd)"

configuration_value() {
    local key="$1"
    sed -n "s/^${key} = \"\\([^\"]*\\)\"$/\\1/p" "$source_configuration"
}

version="$(configuration_value version)"
source_url="$(configuration_value source_url)"
source_sha256="$(configuration_value source_sha256)"
glib_dependencies_url="$(configuration_value glib_dependencies_url)"
glib_dependencies_sha256="$(configuration_value glib_dependencies_sha256)"
released="$(configuration_value released)"
maximum_glibc="$(configuration_value maximum_glibc)"
minimum_gcc="$(configuration_value minimum_gcc)"
smoke_timeout_seconds="$(configuration_value smoke_timeout_seconds)"
cli_wpe_version="$(
    sed -n 's/^wpe_version = "\([^"]*\)"$/\1/p' "$repo_root/cli/src/browser_runtime.toml"
)"

if [[ -z "$version" || -z "$source_url" || -z "$source_sha256" || -z "$glib_dependencies_url" || -z "$glib_dependencies_sha256" || -z "$released" || -z "$maximum_glibc" || -z "$minimum_gcc" || -z "$smoke_timeout_seconds" ]]; then
    echo "invalid WPE runtime source configuration" >&2
    exit 1
fi

if [[ "$cli_wpe_version" != "$version" ]]; then
    echo "WPE runtime source version $version does not match CLI version $cli_wpe_version" >&2
    exit 1
fi

case "$(uname -m)" in
    x86_64)
        architecture="x86_64"
        ;;
    aarch64)
        architecture="aarch64"
        ;;
    *)
        echo "WPE runtime artifacts require native x86_64 or aarch64 Linux" >&2
        exit 1
        ;;
esac

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

actual_glibc="$(getconf GNU_LIBC_VERSION | awk '{print $2}')"
highest_glibc="$(printf '%s\n%s\n' "$actual_glibc" "$maximum_glibc" | sort -V | tail -n 1)"
if [[ "$highest_glibc" != "$maximum_glibc" ]]; then
    echo "build host glibc $actual_glibc exceeds artifact floor $maximum_glibc" >&2
    exit 1
fi

work_directory="$(mktemp -d)"
trap 'rm -rf "$work_directory"' EXIT
archive="$work_directory/wpewebkit.tar.xz"
curl --fail --location --retry 3 --output "$archive" "$source_url"
printf '%s  %s\n' "$source_sha256" "$archive" | sha256sum --check
tar -xJf "$archive" -C "$work_directory"
source_directory="$work_directory/wpewebkit-$version"
build_directory="$work_directory/build"
prefix="$work_directory/runtime"

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
    -DUSE_LIBBACKTRACE=OFF
cmake --build "$build_directory" --parallel "${WATERUI_WPE_BUILD_JOBS:-$(nproc)}"
cmake --install "$build_directory"

bridge_build_directory="$work_directory/bridge-build"
PKG_CONFIG_PATH="$prefix/lib/pkgconfig:$prefix/share/pkgconfig" \
cmake \
    -S "$repo_root/components/platform/browser-wpe/native" \
    -B "$bridge_build_directory" \
    -G Ninja \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="$prefix" \
    -DCMAKE_C_COMPILER="$c_compiler" \
    -DCMAKE_CXX_COMPILER="$cxx_compiler" \
    -DCMAKE_PREFIX_PATH="$prefix"
cmake --build "$bridge_build_directory"
cmake --install "$bridge_build_directory"

python3 "$runtime_directory/package-runtime.py" \
    --architecture "$architecture" \
    --maximum-glibc "$maximum_glibc" \
    --output "$output_directory" \
    --prefix "$prefix" \
    --released "$released" \
    --repository "$repo_root" \
    --source "$source_directory" \
    --version "$version"

cargo build \
    --package waterui-browser-wpe \
    --example runtime_smoke \
    --features runtime-smoke
smoke_binary="$repo_root/target/debug/examples/runtime_smoke"
archive="$output_directory/waterui-wpe-$version-linux-$architecture.zip"
snapshot="$output_directory/wpe-smoke-$version-linux-$architecture.png"
metrics="$output_directory/wpe-smoke-$version-linux-$architecture.json"
python3 "$runtime_directory/run-smoke.py" \
    --archive "$archive" \
    --binary "$smoke_binary" \
    --metrics "$metrics" \
    --runtime "$prefix" \
    --snapshot "$snapshot" \
    --timeout-seconds "$smoke_timeout_seconds"
