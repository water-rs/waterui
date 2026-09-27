#!/usr/bin/env bash
# Stages what the non-host expansions need on a macOS host. Each expansion is
# a check build — nothing links — so every probe only needs its header set,
# a pkg-config answer, or a compiler that parses:
#
#   * `x86_64-unknown-linux-gnu`: pipewire-sys / libspa-sys read PipeWire and
#     SPA headers through `pkg-config` (they are plain source-tree headers),
#     alsa-sys / khronos-egl probe `alsa` / `egl` for link flags only, bindgen
#     wants a libc header set — musl's is self-contained — and `cc`-based
#     build scripts (ring) want a `x86_64-linux-gnu-gcc`, which clang answers
#     with `--target`.
#   * `aarch64-linux-android`: ring wants `aarch64-linux-android<api>-clang`
#     (the same clang trick; the musl sysroot stands in for Bionic because
#     nothing is linked or run), and the WaterKit shims compile Kotlin with
#     `kotlinc` against the runner's Android SDK.
#
# Everything lands under one scratch directory and is wired through
# environment variables, so nothing outside it is modified.

set -euo pipefail

work="$(mktemp -d /tmp/waterui-cross-deps.XXXXXX)"
echo "Provisioning cross-target expansion dependencies in $work"

# --- libc headers for bindgen ------------------------------------------------
curl -fsSL https://musl.libc.org/releases/musl-1.2.5.tar.gz | tar xz -C "$work"
musl="$work/musl-1.2.5"
libc="$work/linux-libc/include"
mkdir -p "$libc/bits"
cp -R "$musl/include/." "$libc/"
cp -R "$musl/arch/generic/bits/." "$libc/bits/"
cp -R "$musl/arch/x86_64/bits/." "$libc/bits/"
cp -R "$musl/arch/x86_64/." "$libc/"
sed -f "$musl/tools/mkalltypes.sed" \
    "$musl/arch/x86_64/bits/alltypes.h.in" "$musl/include/alltypes.h.in" \
    > "$libc/bits/alltypes.h"
grep -v '^#' "$musl/arch/x86_64/bits/syscall.h.in" \
    | sed 's/__NR_/SYS_/g' > "$libc/bits/syscall.h"
echo '1.2.5' > "$libc/bits/version.h"

# --- PipeWire / SPA headers ---------------------------------------------------
curl -fsSL \
    https://gitlab.freedesktop.org/pipewire/pipewire/-/archive/1.4.7/pipewire-1.4.7.tar.gz \
    | tar xz -C "$work"
pipewire="$work/pipewire-1.4.7"
sysroot="$work/linux-headers"
mkdir -p "$sysroot/pipewire" "$sysroot/spa-0.2"
cp "$pipewire/src/pipewire/"*.h "$sysroot/pipewire/"
cp -R "$pipewire/src/pipewire/extensions" "$sysroot/pipewire/"
cp -R "$pipewire/spa/include/spa" "$sysroot/spa-0.2/"
sed \
    -e 's/@PIPEWIRE_VERSION_MAJOR@/1/g' \
    -e 's/@PIPEWIRE_VERSION_MINOR@/4/g' \
    -e 's/@PIPEWIRE_VERSION_MICRO@/7/g' \
    -e 's/@PIPEWIRE_API_VERSION@/0.5/g' \
    "$pipewire/src/pipewire/version.h.in" > "$sysroot/pipewire/version.h"

# --- pkg-config answers for the probed libraries ------------------------------
pcdir="$work/pkgconfig"
mkdir -p "$pcdir"
stub_pc() {
    local name="$1" version="$2" extra_include="${3:-}"
    cat > "$pcdir/$name.pc" <<EOF
prefix=$sysroot
libdir=\${prefix}
includedir=\${prefix}$extra_include

Name: $name
Description: Header-only stub for the header generator's check expansion
Version: $version
Libs: -L\${libdir}
Cflags: -I\${includedir}
EOF
}
stub_pc libspa-0.2 0.2 "/spa-0.2"
stub_pc libpipewire-0.3 1.4.7
# pipewire.h includes <spa/...> and pipewire-sys only forwards libpipewire's
# own include paths to bindgen, so the dependency has to be real.
sed -i '' '/^Description:/a\
Requires: libspa-0.2
' "$pcdir/libpipewire-0.3.pc"
stub_pc alsa 1.2.10
stub_pc egl 1.5

# --- the cc probes ------------------------------------------------------------
bindir="$work/bin"
mkdir -p "$bindir"
cat > "$bindir/x86_64-linux-gnu-gcc" <<EOF
#!/bin/sh
exec clang --target=x86_64-unknown-linux-gnu -isystem "$libc" "\$@"
EOF
chmod +x "$bindir/x86_64-linux-gnu-gcc"

# `cc-rs` looks the Android compiler up by API-suffixed name first, so provide
# both spellings. The API floor the wrappers are named for is read from the
# root manifest, the same place `cargo metadata` finds it.
api="$(cargo metadata --format-version 1 --no-deps \
    | jq -r '.packages[] | select(.name == "waterui") | .metadata.waterui["android-min-api-level"] | numbers')"
for name in "aarch64-linux-android${api}-clang" "aarch64-linux-android${api}-clang++" \
            "aarch64-linux-android-clang" "aarch64-linux-android-clang++"; do
    cat > "$bindir/$name" <<EOF
#!/bin/sh
exec clang --target=aarch64-linux-android --sysroot="$work/android-sysroot" "\$@"
EOF
    chmod +x "$bindir/$name"
done
mkdir -p "$work/android-sysroot/usr"
ln -sfn "$libc" "$work/android-sysroot/usr/include"
cat > "$bindir/aarch64-linux-android-ar" <<'EOF'
#!/bin/sh
exec ar "$@"
EOF
chmod +x "$bindir/aarch64-linux-android-ar"

{
    echo "PKG_CONFIG_PATH=$pcdir"
    echo "PKG_CONFIG_ALLOW_CROSS=1"
    echo "BINDGEN_EXTRA_CLANG_ARGS=-nostdinc -isystem $libc"
    echo "PATH=$bindir:$PATH"
} >> "$GITHUB_ENV"
