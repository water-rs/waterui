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
setlocal EnableDelayedExpansion
set "tool=%~n0"

rem Defaults matching the .sh `${VAR:-default}` expansions; a declared value
rem always wins.
if not defined WATERUI_FAKE_RUSTC_VERSION set "WATERUI_FAKE_RUSTC_VERSION=1.0.0"
if not defined WATERUI_FAKE_CARGO_VERSION set "WATERUI_FAKE_CARGO_VERSION=1.95.0"
if not defined WATERUI_FAKE_XCODE_VERSION set "WATERUI_FAKE_XCODE_VERSION=16.4"
if not defined WATERUI_FAKE_ADB_VERSION set "WATERUI_FAKE_ADB_VERSION=36.0.0-test"
if not defined WATERUI_FAKE_KOTLINC_VERSION set "WATERUI_FAKE_KOTLINC_VERSION=0.0.0"
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
if /i "%tool%"=="xcodebuild" goto :xcodebuild
if /i "%tool%"=="xcode-select" goto :xcode_select
if /i "%tool%"=="xcrun" goto :xcrun
if /i "%tool%"=="sdkmanager" goto :sdkmanager
if /i "%tool%"=="adb" goto :adb
if /i "%tool%"=="emulator" goto :emulator
if /i "%tool%"=="java" goto :exit_ok
if /i "%tool%"=="javac" goto :exit_ok
if /i "%tool%"=="kotlinc" goto :kotlinc
if /i "%tool%"=="cmake" goto :simple_version
if /i "%tool%"=="meson" goto :simple_version
if /i "%tool%"=="sccache" goto :simple_version
if /i "%tool%"=="wasm-pack" goto :simple_version
if /i "%tool%"=="sh" goto :simple_version
if /i "%tool%"=="bash" goto :simple_version
if /i "%tool%"=="brew" goto :exit_ok
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

:exit_ok
exit /b 0

:rustup
if "%*"=="show active-toolchain" (
    if defined WATERUI_FAKE_RUSTUP_NO_ACTIVE_TOOLCHAIN (echo error: no active toolchain 1>&2 & exit /b 1)
    call :respond RUSTUP_ACTIVE_TOOLCHAIN & exit /b !errorlevel!
)
if "%*"=="target list --installed" (call :respond_or_empty RUSTUP_INSTALLED_TARGETS & exit /b 0)
set "args=%*"
if "!args:~0,17!"=="toolchain install" exit /b 0
if "!args:~0,7!"=="update " exit /b 0
if "!args:~0,10!"=="target add" exit /b 0
if "%1"=="--version" (echo rustup 1.28.2 (waterui-test) & exit /b 0)
if "%1"=="-V" (echo rustup 1.28.2 (waterui-test) & exit /b 0)
exit /b 2

:rustc
if "%1"=="--version" (echo rustc %WATERUI_FAKE_RUSTC_VERSION% (waterui-test 2026-01-01) & exit /b 0)
if "%1"=="-vV" (
    if defined WATERUI_FAKE_RUSTC_HOST (
        echo rustc %WATERUI_FAKE_RUSTC_VERSION% (waterui-test)
        echo binary: rustc
        echo commit-hash: fake
        echo commit-date: 2026-01-01
        echo host: %WATERUI_FAKE_RUSTC_HOST%
        echo release: %WATERUI_FAKE_RUSTC_VERSION%
        echo LLVM version: 20.1.0
        exit /b 0
    )
    call :respond RUSTC_VERBOSE & exit /b !errorlevel!
)
exit /b 2

:cargo
if "%1"=="--version" echo cargo %WATERUI_FAKE_CARGO_VERSION% (waterui-test)
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
if "%1"=="-version" (echo info: kotlinc-jvm %WATERUI_FAKE_KOTLINC_VERSION% (JRE 17.0.0) & exit /b 0)
exit /b 0

:simple_version
if "%1"=="--version" (echo %tool% 1.0.0 (waterui-test) & exit /b 0)
if "%1"=="-version" (echo %tool% 1.0.0 (waterui-test) & exit /b 0)
if "%1"=="-v" (echo %tool% 1.0.0 (waterui-test) & exit /b 0)
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
