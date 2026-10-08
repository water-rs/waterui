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
# on the child PATH. Everything below is POSIX builtins, except absolute-path
# `/bin/mkdir`/`/bin/cp` calls, which resolve regardless of PATH.
#
# Mutable state (`rustup toolchain install`/`default`/`target add`/
# `component add`, `rustup update`, `cargo install`, `espup install`) lives
# in files under the fake $HOME so a `--fix` run mutates the fixture and a
# re-check observes the repair.
#
# Asymmetry with fake_tools.cmd: `sdkmanager --licenses`/`--install` drain
# stdin here (a `read` loop) so piped license confirmations are consumed.
# cmd.exe has no builtin way to read stdin to EOF (`set /p` reads one line
# and cannot test EOF), so the .cmd returns immediately instead — tests must
# not rely on stdin being drained on Windows.

tool=${0##*/}
tool=${tool%.cmd}
tool=${tool%.bat}
tool=${tool%.exe}

# WATERUI_FAKE_LOG names a file every fake invocation appends itself to —
# `<tool> <args>` one per line — the seam tests use to assert which tools
# ran and with which arguments.
if [ -n "${WATERUI_FAKE_LOG-}" ]; then
    printf '%s' "$tool" >> "$WATERUI_FAKE_LOG"
    for _log_arg in "$@"; do
        printf ' %s' "$_log_arg" >> "$WATERUI_FAKE_LOG"
    done
    printf '\n' >> "$WATERUI_FAKE_LOG"
fi

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

# file_contains <file> <entry>: <entry> is a line of <file>.
file_contains() {
    [ -f "$1" ] || return 1
    while IFS= read -r _entry; do
        [ "$_entry" = "$2" ] && return 0
    done < "$1"
    return 1
}

case "$tool" in
rustup)
    # Mutable rustup state — one channel/target/component per line.
    _st_tc="$HOME/.fake-rustup-toolchains"
    _st_def="$HOME/.fake-rustup-default"
    _st_tgt="$HOME/.fake-rustup-targets"
    _st_cmp="$HOME/.fake-rustup-components"
    _rt_home="${RUSTUP_HOME:-$HOME/.rustup}"
    # The channel a test's `rust-toolchain.toml` pin declares. When set,
    # `show active-toolchain` resolves it from the state above — a pin for a
    # toolchain nothing installed fails like rustup's own error.
    _pin="${WATERUI_FAKE_RUSTUP_TOOLCHAIN_NOT_INSTALLED-}"
    case "$*" in
        "show active-toolchain")
            if [ -n "${WATERUI_FAKE_RUSTUP_NO_ACTIVE_TOOLCHAIN-}" ]; then
                echo "error: no active toolchain" >&2
                exit 1
            fi
            if [ -n "$_pin" ]; then
                if file_contains "$_st_tc" "$_pin" || [ -d "$_rt_home/toolchains/$_pin" ]; then
                    case "$_pin" in
                        stable | beta | nightly | [0-9]*)
                            echo "$_pin-${WATERUI_FAKE_RUSTC_HOST:-x86_64-unknown-fake} (overridden by rust-toolchain.toml)"
                            ;;
                        *)
                            # Custom/linked toolchains (esp, stage0) print bare.
                            echo "$_pin (overridden by rust-toolchain.toml)"
                            ;;
                    esac
                    exit 0
                fi
                echo "error: toolchain '$_pin' is not installed" >&2
                exit 1
            fi
            if [ -f "$_st_def" ]; then
                IFS= read -r _def < "$_st_def"
                echo "$_def-${WATERUI_FAKE_RUSTC_HOST:-x86_64-unknown-fake} (default)"
                exit 0
            fi
            respond RUSTUP_ACTIVE_TOOLCHAIN
            ;;
        "toolchain install "* | "toolchain add "*)
            printf '%s\n' "$last_arg" >> "$_st_tc"
            exit 0
            ;;
        "toolchain list")
            eval "_toolchains=\${WATERUI_FAKE_RUSTUP_TOOLCHAINS-}"
            [ -n "$_toolchains" ] && printf '%s\n' "$_toolchains"
            if [ -f "${WATERUI_FAKE_RESPONSES:-/nonexistent}/RUSTUP_TOOLCHAINS" ]; then
                print_file "$WATERUI_FAKE_RESPONSES/RUSTUP_TOOLCHAINS"
            fi
            [ -f "$_st_tc" ] && print_file "$_st_tc"
            exit 0
            ;;
        "default "*)
            printf '%s\n' "$last_arg" >> "$_st_tc"
            printf '%s\n' "$last_arg" > "$_st_def"
            exit 0
            ;;
        "update" | "update "*)
            # `rustup update <name>` moves a moving-channel toolchain to the
            # newest release — model that as a version jump.
            printf '%s\n' "${WATERUI_FAKE_RUSTC_UPDATED_VERSION:-99.0.0}" \
                > "$HOME/.fake-rustc-version"
            exit 0
            ;;
        "target list --installed"*)
            eval "_targets=\${WATERUI_FAKE_RUSTUP_INSTALLED_TARGETS-}"
            [ -n "$_targets" ] && printf '%s\n' "$_targets"
            if [ -f "${WATERUI_FAKE_RESPONSES:-/nonexistent}/RUSTUP_INSTALLED_TARGETS" ]; then
                print_file "$WATERUI_FAKE_RESPONSES/RUSTUP_INSTALLED_TARGETS"
            fi
            [ -f "$_st_tgt" ] && print_file "$_st_tgt"
            exit 0
            ;;
        "target add "*)
            printf '%s\n' "$last_arg" >> "$_st_tgt"
            exit 0
            ;;
        "component list"*)
            eval "_components=\${WATERUI_FAKE_RUSTUP_INSTALLED_COMPONENTS-}"
            [ -n "$_components" ] && printf '%s\n' "$_components"
            if [ -f "${WATERUI_FAKE_RESPONSES:-/nonexistent}/RUSTUP_INSTALLED_COMPONENTS" ]; then
                print_file "$WATERUI_FAKE_RESPONSES/RUSTUP_INSTALLED_COMPONENTS"
            fi
            [ -f "$_st_cmp" ] && print_file "$_st_cmp"
            exit 0
            ;;
        "component add "*)
            printf '%s\n' "$last_arg" >> "$_st_cmp"
            exit 0
            ;;
        "run "*)
            # `rustup run <toolchain> <tool> <args…>` proxies to the named
            # tool — dispatch to the sibling fake on this scratch PATH.
            shift
            shift
            _run_tool="${0%/*}/$1"
            shift
            exec "$_run_tool" "$@"
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
    _rustc_version="${WATERUI_FAKE_RUSTC_VERSION:-1.0.0}"
    if [ -f "$HOME/.fake-rustc-version" ]; then
        IFS= read -r _rustc_version < "$HOME/.fake-rustc-version"
    fi
    case "$1" in
        --version)
            printf 'rustc %s (waterui-test 2026-01-01)\n' "$_rustc_version"
            exit 0
            ;;
        -vV)
            if [ -n "${WATERUI_FAKE_RUSTC_HOST-}" ]; then
                printf 'rustc %s (waterui-test)\nbinary: rustc\ncommit-hash: fake\ncommit-date: 2026-01-01\nhost: %s\nrelease: %s\nLLVM version: 20.1.0\n' \
                    "$_rustc_version" "$WATERUI_FAKE_RUSTC_HOST" "$_rustc_version"
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
        metadata)
            # `cargo metadata` prints the staged CARGO_METADATA JSON and fails when none is staged.
            respond CARGO_METADATA
            ;;
        tree)
            # The graph is resolved per build target: `cargo tree` answers
            # `CARGO_TREE_<triple>` for each `--target` flag it is passed —
            # a multi-target invocation's union is exactly its per-target
            # answers — and a call naming no `--target` or a triple with no
            # staged response fails, so a host-graph resolution can never
            # pass as the target's.
            _tree_targets=""
            _tree_prev=""
            for _tree_arg in "$@"; do
                if [ "$_tree_prev" = "--target" ]; then
                    _tree_targets="$_tree_targets $_tree_arg"
                fi
                _tree_prev="$_tree_arg"
            done
            if [ -z "$_tree_targets" ]; then
                exit 2
            fi
            for _tree_target in $_tree_targets; do
                if [ ! -f "${WATERUI_FAKE_RESPONSES:-/nonexistent}/CARGO_TREE_$_tree_target" ]; then
                    exit 1
                fi
            done
            for _tree_target in $_tree_targets; do
                print_file "${WATERUI_FAKE_RESPONSES}/CARGO_TREE_$_tree_target"
            done
            exit 0
            ;;
        install | binstall)
            # `cargo install <crate>` drops the crate's binary beside cargo —
            # model that by copying this dispatcher under the crate's name.
            shift
            _krate=""
            for _arg in "$@"; do
                case "$_arg" in
                    -*) ;;
                    *)
                        _krate="$_arg"
                        break
                        ;;
                esac
            done
            if [ -n "$_krate" ]; then
                _dest="${0%/*}/$_krate"
                [ -e "$_dest" ] || /bin/cp "$0" "$_dest"
            fi
            ;;
    esac
    exit 0
    ;;
espup)
    case "$*" in
        "install"*)
            # `espup install` lays down the `esp` toolchain's pieces; the
            # RISC-V GCC lands under ~/.espressif only with --esp-riscv-gcc.
            _esp="${RUSTUP_HOME:-$HOME/.rustup}/toolchains/esp"
            /bin/mkdir -p \
                "$_esp/xtensa-esp32-elf-clang/1.0/esp-clang/lib" \
                "$_esp/xtensa-esp-elf/1.0/xtensa-esp-elf/bin" \
                "$_esp/lib/rustlib/src/rust"
            case "$*" in
                *--esp-riscv-gcc*)
                    /bin/mkdir -p "$HOME/.espressif/tools/riscv32-esp-elf/1.0/riscv32-esp-elf/bin"
                    ;;
            esac
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
security)
    case "$*" in
        "find-identity -v -p codesigning")
            respond_or_empty SECURITY_FIND_IDENTITY
            ;;
        "find-certificate -a -Z -p"*)
            respond_or_empty SECURITY_FIND_CERTIFICATE
            ;;
        *)
            exit 0
            ;;
    esac
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
        "devicectl device info processes "*)
            respond XCRUN_DEVICE_INFO_PROCESSES
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
    # Every invocation appends its argv to the log a test points
    # `WATERUI_FAKE_ADB_LOG` at, so sequences can be asserted.
    if [ -n "${WATERUI_FAKE_ADB_LOG-}" ]; then
        printf '%s\n' "$*" >> "$WATERUI_FAKE_ADB_LOG"
    fi
    # A wedged transport — spin until the caller's bound kills the process.
    if [ -n "${WATERUI_FAKE_ADB_HANG-}" ]; then
        while :; do :; done
    fi
    case "$*" in
        version)
            printf 'Android Debug Bridge version 1.0.41\nVersion %s\n' "${WATERUI_FAKE_ADB_VERSION:-36.0.0-test}"
            exit 0
            ;;
        "devices -l")
            printf 'List of devices attached\n'
            respond_or_empty ADB_DEVICES
            ;;
        start-server)
            exit "${WATERUI_FAKE_ADB_START_SERVER_STATUS:-0}"
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
        *"pm list packages"*)
            respond_or_empty ADB_PM_PACKAGES
            ;;
        *" install "*)
            # A failed install still prints its `Failure […]` text before
            # the exit status — `respond_or_empty` exits 0 itself, so it
            # runs in a subshell and the status is this branch's own.
            (respond_or_empty ADB_INSTALL)
            exit "${WATERUI_FAKE_ADB_INSTALL_STATUS:-0}"
            ;;
        *logcat*)
            respond_or_empty ADB_LOGCAT
            ;;
        *"run-as"*cat*)
            respond_or_empty ADB_CAT
            ;;
        *"run-as"*)
            exit "${WATERUI_FAKE_ADB_RUN_AS_STATUS:-0}"
            ;;
        *shell*date*)
            printf '%s\n' "${WATERUI_FAKE_ADB_DATE:-01-01 00:00:00.000}"
            ;;
        *shell*instrument*)
            respond_or_empty ADB_AM_INSTRUMENT
            ;;
        *shell*)
            exit "${WATERUI_FAKE_ADB_SHELL_STATUS:-0}"
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
    case "$*" in
        -version | --version)
            # Real java prints `openjdk version "X"` on stderr; the check
            # reads combined output, so stdout works the same.
            printf 'openjdk version "%s" 2026-01-01\n' "${WATERUI_FAKE_JAVA_VERSION:-99.0.0}"
            ;;
    esac
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
sccache)
    case "$*" in
        --version | -version | -v)
            printf 'sccache %s (waterui-test)\n' "${WATERUI_FAKE_SCCACHE_VERSION:-1.0.0}"
            ;;
    esac
    exit 0
    ;;
git)
    # The managed-checkout sequence `git -C <dir> init | remote add | fetch |
    # checkout` produces: on `checkout`, copy the staged
    # WATERUI_FAKE_GIT_CHECKOUT tree into the -C directory, like a clone's
    # detached checkout. Every other subcommand succeeds silently.
    git_dir=""
    git_prev=""
    for git_arg in "$@"; do
        if [ "$git_prev" = "-C" ]; then git_dir=$git_arg; fi
        git_prev=$git_arg
    done
    case "$*" in
        "--version")
            printf 'git version %s\n' "${WATERUI_FAKE_GIT_VERSION:-2.43.0}"
            ;;
        *checkout*)
            if [ -n "${WATERUI_FAKE_GIT_CHECKOUT-}" ] && [ -n "$git_dir" ]; then
                /bin/cp -R "${WATERUI_FAKE_GIT_CHECKOUT}/." "$git_dir/"
            fi
            ;;
    esac
    exit 0
    ;;
cmake | meson | wasm-pack | spirv-opt | sh | bash)
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
vswhere)
    # `-products * -requires ... -property installationPath -latest` answers
    # with the install path, or nothing when no VC.Tools install exists.
    respond_or_empty VSWHERE
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
