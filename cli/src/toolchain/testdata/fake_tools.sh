#!/bin/sh
# waterui-cli test fixture — fake host tools for Host-driven checks.
#
# A test installs copies of this script under one or more tool names on a
# scratch PATH; the script dispatches on the invoked name. Canned output comes
# from WATERUI_FAKE_<KEY> environment variables or files under
# ${WATERUI_FAKE_RESPONSES}/<key>.
#
# The host PATH under test contains ONLY the fixture bin directory, so this
# script must not invoke external commands (`cat`, `tr`, ...) — they are not
# on the child PATH. Everything below is POSIX builtins.

tool=${0##*/}
tool=${tool%.cmd}
tool=${tool%.bat}
tool=${tool%.exe}

# print_file <path>: emit a file's contents using only builtins. `|| [ -n ... ]`
# keeps a final unterminated line.
print_file() {
    while IFS= read -r _line || [ -n "$_line" ]; do
        printf '%s\n' "$_line"
    done < "$1"
}

# respond <key>: print the canned text for <key>; exit 1 when none exists.
# <key> must be a valid shell variable suffix ([A-Za-z0-9_] only) for the
# env-var branch; callers whose keys can contain '-' or '.' (pkg-config
# module names) use respond_file.
respond() {
    eval "value=\${WATERUI_FAKE_$1-}"
    if [ -n "$value" ]; then
        printf '%s\n' "$value"
        exit 0
    fi
    respond_file "$1"
}

# respond_file <key>: print the response file for <key>; exit 1 when absent.
respond_file() {
    if [ -f "${WATERUI_FAKE_RESPONSES:-/nonexistent}/$1" ]; then
        print_file "${WATERUI_FAKE_RESPONSES}/$1"
        exit 0
    fi
    exit 1
}

# respond_or_empty <key>: like respond, but prints nothing and exits 0 when
# no canned text exists.
respond_or_empty() {
    eval "value=\${WATERUI_FAKE_$1-}"
    if [ -n "$value" ]; then
        printf '%s\n' "$value"
        exit 0
    fi
    if [ -f "${WATERUI_FAKE_RESPONSES:-/nonexistent}/$1" ]; then
        print_file "${WATERUI_FAKE_RESPONSES}/$1"
    fi
    exit 0
}

# last_arg evaluates to the final positional argument after this loop.
for last_arg; do :; done

# list_contains <list> <entry>: <entry> is a whitespace-separated member.
list_contains() {
    case " $1 " in
        *" $2 "*) return 0 ;;
        *) return 1 ;;
    esac
}

# module_present <name>: a pkg-config module response file exists for <name>.
# Module names can contain '-' and '.', which are not expandable inside ${}
# on every /bin/sh, so module state lives in files only.
module_present() {
    [ -f "${WATERUI_FAKE_RESPONSES:-/nonexistent}/PKG_CONFIG_$1" ]
}

case "$tool" in
rustup)
    case "$*" in
        "show active-toolchain")
            if [ -n "${WATERUI_FAKE_RUSTUP_NO_ACTIVE_TOOLCHAIN-}" ]; then
                echo "error: no active toolchain" >&2
                exit 1
            fi
            respond RUSTUP_ACTIVE_TOOLCHAIN
            ;;
        "target list --installed")
            respond_or_empty RUSTUP_INSTALLED_TARGETS
            ;;
        "toolchain install"* | "update "* | "target add"*)
            exit 0
            ;;
        --version | -V)
            echo "rustup 1.28.2 (waterui-test)"
            exit 0
            ;;
        *)
            exit 2
            ;;
    esac
    ;;
rustc)
    case "$1" in
        --version)
            printf 'rustc %s (waterui-test 2026-01-01)\n' "${WATERUI_FAKE_RUSTC_VERSION:-1.0.0}"
            exit 0
            ;;
        -vV)
            if [ -n "${WATERUI_FAKE_RUSTC_HOST-}" ]; then
                printf 'rustc %s (waterui-test)\nbinary: rustc\ncommit-hash: fake\ncommit-date: 2026-01-01\nhost: %s\nrelease: %s\nLLVM version: 20.1.0\n' \
                    "${WATERUI_FAKE_RUSTC_VERSION:-1.0.0}" "$WATERUI_FAKE_RUSTC_HOST" \
                    "${WATERUI_FAKE_RUSTC_VERSION:-1.0.0}"
                exit 0
            fi
            respond RUSTC_VERBOSE
            ;;
        *)
            exit 2
            ;;
    esac
    ;;
cargo)
    case "$1" in
        --version)
            printf 'cargo %s (waterui-test)\n' "${WATERUI_FAKE_CARGO_VERSION:-1.95.0}"
            ;;
    esac
    exit 0
    ;;
xcodebuild)
    case "$*" in
        -version | --version)
            printf 'Xcode %s\nBuild version 16F6\n' "${WATERUI_FAKE_XCODE_VERSION:-16.4}"
            ;;
    esac
    exit 0
    ;;
xcode-select)
    case "$*" in
        -p)
            respond_or_empty XCODE_SELECT_DEVELOPER_DIR
            ;;
    esac
    exit 0
    ;;
xcrun)
    case "$*" in
        *--show-sdk-path)
            respond XCRUN_SDK_PATH
            ;;
        "simctl list --json")
            respond XCRUN_SIMCTL_DEVICES
            ;;
        "simctl delete unavailable" | "simctl create "*)
            exit 0
            ;;
        *)
            exit 1
            ;;
    esac
    ;;
sdkmanager)
    case "$*" in
        *--licenses* | *--install*)
            # Drain piped stdin (license confirmations) with builtins.
            while IFS= read -r _drain; do :; done
            exit 0
            ;;
        *--list*)
            respond_or_empty SDKMANAGER_LIST
            ;;
        *--version*)
            echo "5.0"
            exit 0
            ;;
        *)
            exit 0
            ;;
    esac
    ;;
adb)
    case "$*" in
        version)
            printf 'Android Debug Bridge version 1.0.41\nVersion %s\n' "${WATERUI_FAKE_ADB_VERSION:-36.0.0-test}"
            exit 0
            ;;
        "devices -l")
            printf 'List of devices attached\n'
            respond_or_empty ADB_DEVICES
            ;;
        *"emu avd name")
            respond_or_empty ADB_EMU_AVD_NAME
            ;;
        *getprop*)
            respond_or_empty ADB_GETPROP
            ;;
        *wait-for-device*)
            exit 0
            ;;
        *)
            exit 0
            ;;
    esac
    ;;
emulator)
    case "$*" in
        *-list-avds*)
            respond_or_empty EMULATOR_AVDS
            ;;
        *)
            exit 0
            ;;
    esac
    ;;
java | javac)
    exit 0
    ;;
kotlinc)
    case "$*" in
        -version)
            printf 'info: kotlinc-jvm %s (JRE 17.0.0)\n' "${WATERUI_FAKE_KOTLINC_VERSION:-0.0.0}"
            ;;
    esac
    exit 0
    ;;
cmake | meson | sccache | wasm-pack | sh | bash)
    case "$*" in
        --version | -version | -v)
            printf '%s 1.0.0 (waterui-test)\n' "$tool"
            ;;
    esac
    exit 0
    ;;
brew)
    exit 0
    ;;
winget)
    case "$1 $2" in
        "list --id")
            if list_contains "${WATERUI_FAKE_WINGET_INSTALLED-}" "$3"; then
                exit 0
            fi
            exit 1
            ;;
        install*)
            exit 0
            ;;
        *)
            exit 2
            ;;
    esac
    ;;
apt-get | dnf | zypper | sudo)
    exit 0
    ;;
dpkg-query)
    case "$1" in
        -W)
            if list_contains "${WATERUI_FAKE_DPKG_INSTALLED-}" "$last_arg"; then
                echo "install ok installed"
                exit 0
            fi
            exit 1
            ;;
        *)
            exit 1
            ;;
    esac
    ;;
dpkg)
    case "$1" in
        --print-foreign-architectures)
            respond_or_empty DPKG_FOREIGN_ARCHES
            ;;
    esac
    exit 0
    ;;
rpm)
    case "$1" in
        -q)
            if list_contains "${WATERUI_FAKE_RPM_INSTALLED-}" "$last_arg"; then
                exit 0
            fi
            exit 1
            ;;
        *)
            exit 1
            ;;
    esac
    ;;
pacman)
    case "$1" in
        -Q)
            if list_contains "${WATERUI_FAKE_PACMAN_INSTALLED-}" "$last_arg"; then
                printf '%s 1.0\n' "$last_arg"
                exit 0
            fi
            exit 1
            ;;
        *)
            exit 0
            ;;
    esac
    ;;
apk)
    case "$1 $2" in
        "info -e")
            if list_contains "${WATERUI_FAKE_APK_INSTALLED-}" "$last_arg"; then
                exit 0
            fi
            exit 1
            ;;
        *)
            exit 0
            ;;
    esac
    ;;
pkg-config)
    case "$1" in
        --version)
            echo "1.8.1"
            exit 0
            ;;
        --exists)
            module_present "$2" || exit 1
            exit 0
            ;;
        --atleast-version=*)
            module_present "$2" || exit 1
            if list_contains "${WATERUI_FAKE_PKG_CONFIG_TOO_OLD-}" "$2"; then
                exit 1
            fi
            exit 0
            ;;
        --modversion)
            respond_file "PKG_CONFIG_$2"
            ;;
        --variable=*)
            respond "PKG_CONFIG_VAR_${1#--variable=}"
            ;;
        *)
            exit 0
            ;;
    esac
    ;;
*-clang | *-clang++* | clang | clang++ | ld | ld64)
    exit 0
    ;;
uname)
    case "$*" in
        -m)
            echo "${WATERUI_FAKE_UNAME_MACHINE:-x86_64}"
            ;;
        *)
            echo "WaterUITest"
            ;;
    esac
    exit 0
    ;;
*)
    exit 0
    ;;
esac
