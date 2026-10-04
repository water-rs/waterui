@echo off
rem waterui-cli test fixture - fake host tools for Host-driven checks.
rem Windows counterpart of fake_tools.sh; keep dispatch tables in sync.
rem
rem The host PATH under test contains ONLY the fixture bin directory, so this
rem script must not invoke external commands (`findstr`, ...). Everything
rem below is cmd builtins. pkg-config module keys are file-only because '-'
rem and '.' cannot appear in a sh ${} expansion on Unix.
rem
rem Asymmetry with fake_tools.sh: `sdkmanager --licenses`/`--install` cannot
rem drain stdin here - cmd has no builtin way to read stdin to EOF (`set /p`
rem reads one line and cannot test EOF) - so this script returns immediately
rem while the .sh consumes piped license confirmations first. Tests must not
rem rely on stdin being drained on Windows.
rem
rem Mutable state (`rustup toolchain install`/`default`/`target add`/
rem `component add`, `rustup update`, `cargo install`, `espup install`) lives
rem in files under the fake %HOME% so a `--fix` run mutates the fixture and a
rem re-check observes the repair. `mkdir`/`copy`/`type`/`for /f` are cmd
rem builtins, so the restricted PATH is still honored.
setlocal EnableDelayedExpansion
set "tool=%~n0"

rem Defaults matching the .sh `${VAR:-default}` expansions; a declared value
rem always wins.
if not defined WATERUI_FAKE_RUSTC_VERSION set "WATERUI_FAKE_RUSTC_VERSION=1.0.0"
if not defined WATERUI_FAKE_RUSTC_UPDATED_VERSION set "WATERUI_FAKE_RUSTC_UPDATED_VERSION=99.0.0"
if not defined WATERUI_FAKE_CARGO_VERSION set "WATERUI_FAKE_CARGO_VERSION=1.95.0"
if not defined WATERUI_FAKE_XCODE_VERSION set "WATERUI_FAKE_XCODE_VERSION=16.4"
if not defined WATERUI_FAKE_ADB_VERSION set "WATERUI_FAKE_ADB_VERSION=36.0.0-test"
if not defined WATERUI_FAKE_KOTLINC_VERSION set "WATERUI_FAKE_KOTLINC_VERSION=0.0.0"
if not defined WATERUI_FAKE_SCCACHE_VERSION set "WATERUI_FAKE_SCCACHE_VERSION=1.0.0"
if not defined WATERUI_FAKE_UNAME_MACHINE set "WATERUI_FAKE_UNAME_MACHINE=x86_64"
goto :dispatch

rem ------------------------------------------------------------------
rem :respond <key> - echo canned text; exit /b 1 when none exists.
:respond
call set "v=%%WATERUI_FAKE_%~1%%"
if defined v (echo !v! & exit /b 0)
if exist "%WATERUI_FAKE_RESPONSES%\%~1" (type "%WATERUI_FAKE_RESPONSES%\%~1" & exit /b 0)
exit /b 1

rem :respond_file <key> - echo the response file only; exit /b 1 when absent.
:respond_file
if exist "%WATERUI_FAKE_RESPONSES%\%~1" (type "%WATERUI_FAKE_RESPONSES%\%~1" & exit /b 0)
exit /b 1

rem :respond_or_empty <key> - like :respond, exit /b 0 when absent.
:respond_or_empty
call set "v=%%WATERUI_FAKE_%~1%%"
if defined v (echo !v!)
if exist "%WATERUI_FAKE_RESPONSES%\%~1" (type "%WATERUI_FAKE_RESPONSES%\%~1")
exit /b 0

rem :last_arg - set last_arg to the final positional argument.
:last_arg
set "last_arg=%~1"
shift
if not "%~1"=="" goto :last_arg
exit /b 0

rem :list_contains <list> <entry> - errorlevel 0 when entry is a member.
rem Builtin-only: remove " entry " from " list "; a change means membership.
:list_contains
set "list= %~1 "
if "!list: %~2 =!"=="!list!" (exit /b 1) else (exit /b 0)

rem :module_exists <name> - errorlevel 0 when a pkg-config module response
rem file is staged for <name> (raw name; file-only, see header note).
:module_exists
if exist "%WATERUI_FAKE_RESPONSES%\PKG_CONFIG_%~1" exit /b 0
exit /b 1

rem :contains <haystack-var-name> <needle> - errorlevel 0 when !haystack!
rem contains <needle>.
:contains
call set "hay=%%%~1%%"
if not "!hay:%~2=!"=="!hay!" (exit /b 0) else (exit /b 1)

:dispatch
if /i "%tool%"=="rustup" goto :rustup
if /i "%tool%"=="rustc" goto :rustc
if /i "%tool%"=="cargo" goto :cargo
if /i "%tool%"=="espup" goto :espup
if /i "%tool%"=="xcodebuild" goto :xcodebuild
if /i "%tool%"=="xcode-select" goto :xcode_select
if /i "%tool%"=="xcrun" goto :xcrun
if /i "%tool%"=="sdkmanager" goto :sdkmanager
if /i "%tool%"=="adb" goto :adb
if /i "%tool%"=="emulator" goto :emulator
if /i "%tool%"=="java" goto :java
if /i "%tool%"=="javac" goto :java
if /i "%tool%"=="kotlinc" goto :kotlinc
if /i "%tool%"=="cmake" goto :simple_version
if /i "%tool%"=="meson" goto :simple_version
if /i "%tool%"=="sccache" goto :sccache
if /i "%tool%"=="git" goto :git
if /i "%tool%"=="wasm-pack" goto :simple_version
if /i "%tool%"=="spirv-opt" goto :simple_version
if /i "%tool%"=="sh" goto :simple_version
if /i "%tool%"=="bash" goto :simple_version
if /i "%tool%"=="brew" goto :exit_ok
if /i "%tool%"=="vswhere" goto :vswhere
if /i "%tool%"=="winget" goto :winget
if /i "%tool%"=="apt-get" goto :exit_ok
if /i "%tool%"=="dnf" goto :exit_ok
if /i "%tool%"=="zypper" goto :exit_ok
if /i "%tool%"=="sudo" goto :exit_ok
if /i "%tool%"=="dpkg-query" goto :dpkg_query
if /i "%tool%"=="dpkg" goto :dpkg
if /i "%tool%"=="rpm" goto :rpm
if /i "%tool%"=="pacman" goto :pacman
if /i "%tool%"=="apk" goto :apk
if /i "%tool%"=="pkg-config" goto :pkg_config
if not "%tool%"=="%tool:clang=%" goto :exit_ok
if /i "%tool%"=="ld" goto :exit_ok
if /i "%tool%"=="ld64" goto :exit_ok
if /i "%tool%"=="uname" goto :uname
goto :exit_ok

:java
rem Real java prints `openjdk version "X"` on stderr; the check reads
rem combined output, so stdout works the same.
if not defined WATERUI_FAKE_JAVA_VERSION set "WATERUI_FAKE_JAVA_VERSION=99.0.0"
if "%~1"=="-version" echo openjdk version "%WATERUI_FAKE_JAVA_VERSION%" 2026-01-01
if "%~1"=="--version" echo openjdk version "%WATERUI_FAKE_JAVA_VERSION%" 2026-01-01
goto :exit_ok

:exit_ok
exit /b 0

rem The managed-checkout sequence `git -C <dir> init | remote add | fetch |
rem checkout` produces: on `checkout`, copy every file the staged
rem WATERUI_FAKE_GIT_CHECKOUT tree holds into the -C directory, like a
rem clone's detached checkout. `for /r`, `mkdir` and `copy` are builtins, so
rem the restricted PATH is still honored.
:git
set "git_dir="
set "git_prev="
for %%a in (%*) do (
    if /i "!git_prev!"=="-C" set "git_dir=%%~a"
    set "git_prev=%%~a"
)
if "%~1"=="--version" (echo git version 2.43.0 & exit /b 0)
set "git_args=%*"
call :contains git_args checkout
if %errorlevel%==0 if defined WATERUI_FAKE_GIT_CHECKOUT if defined git_dir (
    for /r "%WATERUI_FAKE_GIT_CHECKOUT%" %%f in (*) do (
        set "git_abs=%%f"
        set "git_rel=!git_abs:%WATERUI_FAKE_GIT_CHECKOUT%\=!"
        for %%d in ("!git_dir!\!git_rel!") do if not exist "%%~dpd" mkdir "%%~dpd"
        copy /y "!git_abs!" "!git_dir!\!git_rel!" >nul
    )
)
exit /b 0

:rustup
rem Mutable rustup state - one channel/target/component per line in the
rem state files, plus a directory per installed toolchain under
rem %RUSTUP_HOME%\toolchains, the same layout a real rustup produces.
set "state_toolchains=%HOME%\.fake-rustup-toolchains"
set "state_default=%HOME%\.fake-rustup-default"
set "state_targets=%HOME%\.fake-rustup-targets"
set "state_components=%HOME%\.fake-rustup-components"
if defined RUSTUP_HOME (set "rustup_home=%RUSTUP_HOME%") else (set "rustup_home=%HOME%\.rustup")
if not defined WATERUI_FAKE_RUSTC_HOST set "WATERUI_FAKE_RUSTC_HOST=x86_64-unknown-fake"
if "%*"=="show active-toolchain" goto :rustup_active_toolchain
set "args=%*"
if not defined args exit /b 2
for %%a in (%*) do set "last_arg=%%a"
rem Prefix matches below mirror the .sh globs: `rustup <verb> ...` qualified
rem with `--toolchain <name>` must match the same branch as the bare form.
if "!args:~0,17!"=="toolchain install" goto :rustup_install
if "!args:~0,13!"=="toolchain add" goto :rustup_install
if "!args:~0,14!"=="toolchain list" (
    if defined WATERUI_FAKE_RUSTUP_TOOLCHAINS echo !WATERUI_FAKE_RUSTUP_TOOLCHAINS!
    if exist "%WATERUI_FAKE_RESPONSES%\RUSTUP_TOOLCHAINS" type "%WATERUI_FAKE_RESPONSES%\RUSTUP_TOOLCHAINS"
    if exist "%state_toolchains%" type "%state_toolchains%"
    exit /b 0
)
if "!args:~0,7!"=="default" goto :rustup_default
if "!args:~0,6!"=="update" (>"%HOME%\.fake-rustc-version" echo %WATERUI_FAKE_RUSTC_UPDATED_VERSION% & exit /b 0)
if "!args:~0,23!"=="target list --installed" (
    if defined WATERUI_FAKE_RUSTUP_INSTALLED_TARGETS echo !WATERUI_FAKE_RUSTUP_INSTALLED_TARGETS!
    if exist "%WATERUI_FAKE_RESPONSES%\RUSTUP_INSTALLED_TARGETS" type "%WATERUI_FAKE_RESPONSES%\RUSTUP_INSTALLED_TARGETS"
    if exist "%state_targets%" type "%state_targets%"
    exit /b 0
)
if "!args:~0,10!"=="target add" (>>"%state_targets%" echo !last_arg! & exit /b 0)
if "!args:~0,14!"=="component list" (
    if defined WATERUI_FAKE_RUSTUP_INSTALLED_COMPONENTS echo !WATERUI_FAKE_RUSTUP_INSTALLED_COMPONENTS!
    if exist "%WATERUI_FAKE_RESPONSES%\RUSTUP_INSTALLED_COMPONENTS" type "%WATERUI_FAKE_RESPONSES%\RUSTUP_INSTALLED_COMPONENTS"
    if exist "%state_components%" type "%state_components%"
    exit /b 0
)
if "!args:~0,13!"=="component add" (>>"%state_components%" echo !last_arg! & exit /b 0)
if "!args:~0,3!"=="run" goto :rustup_run
if "%1"=="--version" (echo rustup 1.28.2 ^(waterui-test^) & exit /b 0)
if "%1"=="-V" (echo rustup 1.28.2 ^(waterui-test^) & exit /b 0)
exit /b 2

rem `rustup toolchain install`/`add` records the channel and lays down its
rem toolchain directory, like rustup itself.
:rustup_install
>>"%state_toolchains%" echo !last_arg!
mkdir "!rustup_home!\toolchains\!last_arg!" 2>nul
exit /b 0

rem `rustup run <toolchain> <tool> <args...>` proxies to the named tool —
rem dispatch to the sibling fake in this directory, like a real rustup proxy.
rem cmd's `shift` rewrites %0, so the sibling path must be resolved first.
:rustup_run
set "run_tool=%~dp0%~3.cmd"
shift
shift
set "run_args="
:rustup_run_args
if not "%~2"=="" (set "run_args=!run_args! %~2" & shift & goto :rustup_run_args)
call "%run_tool%" %run_args%
exit /b %errorlevel%

rem `rustup default <channel>` installs it if absent and records the default.
:rustup_default
>>"%state_toolchains%" echo !last_arg!
>"%state_default%" echo !last_arg!
mkdir "!rustup_home!\toolchains\!last_arg!" 2>nul
exit /b 0

rem `exit /b` inside a nested parenthesized block does not reach the process
rem exit code through `cmd /c`, so the no-toolchain branch lives at top level.
:rustup_active_toolchain
if defined WATERUI_FAKE_RUSTUP_NO_ACTIVE_TOOLCHAIN echo error: no active toolchain 1>&2
if defined WATERUI_FAKE_RUSTUP_NO_ACTIVE_TOOLCHAIN exit /b 1
if defined WATERUI_FAKE_RUSTUP_TOOLCHAIN_NOT_INSTALLED goto :rustup_pinned_toolchain
set "def_channel="
if exist "%state_default%" for /f "usebackq delims=" %%l in ("%state_default%") do set "def_channel=%%l"
if defined def_channel (echo !def_channel!-%WATERUI_FAKE_RUSTC_HOST% ^(default^) & exit /b 0)
call :respond RUSTUP_ACTIVE_TOOLCHAIN
exit /b %errorlevel%

rem A test's `rust-toolchain.toml` pin names this channel; it resolves once
rem `rustup toolchain install`/`default` recorded it, or a toolchain
rem directory exists (linked/esp-style toolchains live under
rem %RUSTUP_HOME%\toolchains). stable/beta/nightly and version-prefixed
rem channels carry the `-<host>` suffix rustup appends; custom/linked
rem toolchain names (esp, stage0) print bare - the same split as the .sh.
:rustup_pinned_toolchain
set "pin=%WATERUI_FAKE_RUSTUP_TOOLCHAIN_NOT_INSTALLED%"
set "pin_installed="
if exist "%state_toolchains%" for /f "usebackq delims=" %%l in ("%state_toolchains%") do if "%%l"=="!pin!" set "pin_installed=file"
if exist "!rustup_home!\toolchains\!pin!" set "pin_installed=dir"
if not defined pin_installed (
    echo error: toolchain '!pin!' is not installed 1>&2
    exit /b 1
)
set "pin_suffixed="
if /i "!pin!"=="stable" set "pin_suffixed=1"
if /i "!pin!"=="beta" set "pin_suffixed=1"
if /i "!pin!"=="nightly" set "pin_suffixed=1"
for %%d in (0 1 2 3 4 5 6 7 8 9) do if "!pin:~0,1!"=="%%d" set "pin_suffixed=1"
if defined pin_suffixed (echo !pin!-%WATERUI_FAKE_RUSTC_HOST% ^(overridden by rust-toolchain.toml^) & exit /b 0)
echo !pin! (overridden by rust-toolchain.toml)
exit /b 0

:rustc
set "rustc_version=%WATERUI_FAKE_RUSTC_VERSION%"
if exist "%HOME%\.fake-rustc-version" for /f "usebackq delims=" %%v in ("%HOME%\.fake-rustc-version") do set "rustc_version=%%v"
if "%1"=="--version" (echo rustc %rustc_version% ^(waterui-test 2026-01-01^) & exit /b 0)
if "%1"=="-vV" (
    if defined WATERUI_FAKE_RUSTC_HOST (
        echo rustc %rustc_version% ^(waterui-test^)
        echo binary: rustc
        echo commit-hash: fake
        echo commit-date: 2026-01-01
        echo host: %WATERUI_FAKE_RUSTC_HOST%
        echo release: %rustc_version%
        echo LLVM version: 20.1.0
        exit /b 0
    )
    call :respond RUSTC_VERBOSE & exit /b !errorlevel!
)
exit /b 2

:cargo
if not defined WATERUI_FAKE_CARGO_VERSION set "WATERUI_FAKE_CARGO_VERSION=1.95.0"
if "%1"=="--version" (echo cargo %WATERUI_FAKE_CARGO_VERSION% ^(waterui-test^) & exit /b 0)
if "%1"=="install" goto :cargo_install
if "%1"=="binstall" goto :cargo_install
exit /b 0

rem `cargo install <crate>` drops the crate's binary beside cargo - model that
rem by copying this dispatcher under the crate's name.
:cargo_install
set "krate="
for %%a in (%*) do (
    if not defined krate (
        set "arg=%%a"
        if not "!arg:~0,1!"=="-" if not /i "%%a"=="install" if not /i "%%a"=="binstall" set "krate=%%a"
    )
)
if defined krate if not exist "%~dp0!krate!.cmd" copy /y "%~f0" "%~dp0!krate!.cmd" >nul
exit /b 0

rem `espup install` lays down the `esp` toolchain's pieces; the RISC-V GCC
rem lands under %HOME%\.espressif only with --esp-riscv-gcc.
:espup
if not "%1"=="install" exit /b 0
if defined RUSTUP_HOME (set "esp=%RUSTUP_HOME%\toolchains\esp") else (set "esp=%HOME%\.rustup\toolchains\esp")
mkdir "%esp%\xtensa-esp32-elf-clang\1.0\esp-clang\lib" 2>nul
mkdir "%esp%\xtensa-esp-elf\1.0\xtensa-esp-elf\bin" 2>nul
mkdir "%esp%\lib\rustlib\src\rust" 2>nul
set "args=%*"
call :contains args --esp-riscv-gcc && (mkdir "%HOME%\.espressif\tools\riscv32-esp-elf\1.0\riscv32-esp-elf\bin" 2>nul)
exit /b 0

:xcodebuild
set "args=%*"
call :contains args version && (echo Xcode %WATERUI_FAKE_XCODE_VERSION% & echo Build version 16F6 & exit /b 0)
exit /b 0

:xcode_select
if "%1"=="-p" (call :respond_or_empty XCODE_SELECT_DEVELOPER_DIR & exit /b 0)
exit /b 0

:xcrun
set "args=%*"
call :contains args --show-sdk-path && (call :respond XCRUN_SDK_PATH & exit /b !errorlevel!)
if "%*"=="simctl list --json" (call :respond XCRUN_SIMCTL_DEVICES & exit /b !errorlevel!)
if "%*"=="simctl delete unavailable" exit /b 0
if "!args:~0,14!"=="simctl create " exit /b 0
exit /b 1

:sdkmanager
rem Unlike the .sh these branches return without draining stdin - see the
rem header note for the cmd limitation.
set "args=%*"
call :contains args --licenses && exit /b 0
call :contains args --install && exit /b 0
call :contains args --list && (call :respond_or_empty SDKMANAGER_LIST & exit /b 0)
call :contains args --version && (echo 5.0 & exit /b 0)
exit /b 0

:adb
if "%*"=="version" (echo Android Debug Bridge version 1.0.41 & echo Version %WATERUI_FAKE_ADB_VERSION% & exit /b 0)
if "%*"=="devices -l" (echo List of devices attached & call :respond_or_empty ADB_DEVICES & exit /b 0)
if "%*"=="start-server" (if defined WATERUI_FAKE_ADB_START_SERVER_STATUS (exit /b %WATERUI_FAKE_ADB_START_SERVER_STATUS%) else (exit /b 0))
set "args=%*"
call :contains args "emu avd name" && (call :respond_or_empty ADB_EMU_AVD_NAME & exit /b 0)
call :contains args getprop && (call :respond_or_empty ADB_GETPROP & exit /b 0)
call :contains args wait-for-device && exit /b 0
exit /b 0

:emulator
set "args=%*"
call :contains args -list-avds && (call :respond_or_empty EMULATOR_AVDS & exit /b 0)
exit /b 0

:kotlinc
if "%1"=="-version" (echo info: kotlinc-jvm %WATERUI_FAKE_KOTLINC_VERSION% ^(JRE 17.0.0^) & exit /b 0)
exit /b 0

:sccache
if "%1"=="--version" (echo sccache %WATERUI_FAKE_SCCACHE_VERSION% ^(waterui-test^) & exit /b 0)
if "%1"=="-version" (echo sccache %WATERUI_FAKE_SCCACHE_VERSION% ^(waterui-test^) & exit /b 0)
if "%1"=="-v" (echo sccache %WATERUI_FAKE_SCCACHE_VERSION% ^(waterui-test^) & exit /b 0)
exit /b 0

:simple_version
if "%1"=="--version" (echo %tool% 1.0.0 ^(waterui-test^) & exit /b 0)
if "%1"=="-version" (echo %tool% 1.0.0 ^(waterui-test^) & exit /b 0)
if "%1"=="-v" (echo %tool% 1.0.0 ^(waterui-test^) & exit /b 0)
exit /b 0

:vswhere
call :respond_or_empty VSWHERE
exit /b 0

:winget
if "%1 %2"=="list --id" (
    call :list_contains "%WATERUI_FAKE_WINGET_INSTALLED%" "%~3" & exit /b !errorlevel!
)
if "%1"=="install" exit /b 0
exit /b 2

:dpkg_query
if not "%1"=="-W" exit /b 1
call :last_arg %*
call :list_contains "%WATERUI_FAKE_DPKG_INSTALLED%" "%last_arg%" && (echo install ok installed & exit /b 0)
exit /b 1

:dpkg
if "%1"=="--print-foreign-architectures" (call :respond_or_empty DPKG_FOREIGN_ARCHES & exit /b 0)
exit /b 0

:rpm
if not "%1"=="-q" exit /b 1
call :last_arg %*
call :list_contains "%WATERUI_FAKE_RPM_INSTALLED%" "%last_arg%"
exit /b %errorlevel%

:pacman
if not "%1"=="-Q" exit /b 0
call :last_arg %*
call :list_contains "%WATERUI_FAKE_PACMAN_INSTALLED%" "%last_arg%" && (echo %last_arg% 1.0 & exit /b 0)
exit /b 1

:apk
if not "%1 %2"=="info -e" exit /b 0
call :last_arg %*
call :list_contains "%WATERUI_FAKE_APK_INSTALLED%" "%last_arg%"
exit /b %errorlevel%

:pkg_config
rem cmd tokenizes %1-%9 on '=', so `--atleast-version=4.14 gtk4` arrives as
rem three tokens and %2 is the version, not the module. Match the raw %*
rem string and take the module as the last argument, like the .sh's $2.
if "%1"=="--version" (echo 1.8.1 & exit /b 0)
if "%1"=="--exists" (
    call :module_exists "%~2" & exit /b !errorlevel!
)
set "args=%*"
call :contains args "--atleast-version=" && (
    call :last_arg %*
    call :module_exists "!last_arg!" || exit /b 1
    call :list_contains "%WATERUI_FAKE_PKG_CONFIG_TOO_OLD%" "!last_arg!" && exit /b 1
    exit /b 0
)
if "%1"=="--modversion" (
    call :respond_file "PKG_CONFIG_%~2" & exit /b !errorlevel!
)
call :contains args "--variable=" && (
    rem `--variable=prefix gtk4` tokenizes as %1=--variable, %2=prefix,
    rem %3=gtk4 - the variable name is %2.
    call :respond "PKG_CONFIG_VAR_%~2" & exit /b !errorlevel!
)
exit /b 0

:uname
if "%1"=="-m" (echo %WATERUI_FAKE_UNAME_MACHINE% & exit /b 0)
echo WaterUITest
exit /b 0
