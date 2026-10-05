#!/usr/bin/env bash

set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
    echo "usage: $0 <output-directory> [work-directory]" >&2
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
released="$(configuration_value released)"
maximum_glibc="$(configuration_value maximum_glibc)"
minimum_gcc="$(configuration_value minimum_gcc)"
# The runtime this script builds must be the version the `water` CLI
# downloads; that expectation lives in the in-tree CLI's manifest.
cli_wpe_version="$(
    curl --fail --location --retry 3 \
        https://raw.githubusercontent.com/water-rs/waterui/dev/cli/src/browser_runtime.toml \
        | sed -n 's/^wpe_version = "\([^"]*\)"$/\1/p'
)"

if [[ -z "$version" || -z "$released" || -z "$maximum_glibc" || -z "$minimum_gcc" ]]; then
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

# A work directory passed by the caller persists across runs — the layout the
# CI cache restores into. Without one the build stays as ephemeral as before.
if [[ $# -eq 2 ]]; then
    work_directory="$(mkdir -p "$2" && cd "$2" && pwd)"
else
    work_directory="$(mktemp -d)"
    trap 'rm -rf "$work_directory"' EXIT
fi
source_directory="$work_directory/wpewebkit-$version"
prefix="$work_directory/runtime"
stage="$work_directory/stage"

# The WebKit build is the expensive stage by hours. build-webkit.sh skips it
# whenever the install prefix carries its completion marker — which is what
# lets a shim-only change rebuild only the shim, and what the CI cache relies
# on. It also installs the host dependencies the packaging stage vendors
# from, so it runs on every invocation, marker or not.
"$runtime_directory/build-webkit.sh" "$work_directory"

# Everything downstream mutates the tree it works on: the bridge installs its
# shim, package-runtime.py vendors libraries and rewrites rpaths. Stage a
# throwaway copy of the install prefix so the prefix itself stays exactly what
# `cmake --install` produced — re-runs and cache saves see a pristine tree.
rm -rf "$stage"
cp -a "$prefix" "$stage"

bridge_build_directory="$work_directory/bridge-build"
PKG_CONFIG_PATH="$stage/lib/pkgconfig:$stage/share/pkgconfig" \
cmake \
    -S "$repo_root/components/platform/browser-wpe/native" \
    -B "$bridge_build_directory" \
    -G Ninja \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="$stage" \
    -DCMAKE_C_COMPILER="$c_compiler" \
    -DCMAKE_CXX_COMPILER="$cxx_compiler" \
    -DCMAKE_PREFIX_PATH="$stage"
cmake --build "$bridge_build_directory"
cmake --install "$bridge_build_directory"

python3 "$runtime_directory/package-runtime.py" \
    --architecture "$architecture" \
    --maximum-glibc "$maximum_glibc" \
    --output "$output_directory" \
    --prefix "$stage" \
    --released "$released" \
    --repository "$repo_root" \
    --source "$source_directory" \
    --version "$version"
