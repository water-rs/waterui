//! Scratch-machine fixtures for host-driven check tests.
//!
//! A [`TestMachine`] is a temp directory with a `bin/` of dispatcher fake
//! tools (see `testdata/fake_tools.sh` / `fake_tools.cmd`), a `responses/`
//! directory of canned tool output, and a `home/` directory standing in for
//! `$HOME`. [`TestMachine::host`] returns a [`Host`] whose `PATH` is only
//! `bin/` and whose environment is exactly what the fixture declares, so a
//! check under test can never observe real-machine state.

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use tempfile::TempDir;

use super::Host;

#[cfg(unix)]
const FAKE_TOOL_SCRIPT: &str = include_str!("testdata/fake_tools.sh");
#[cfg(windows)]
const FAKE_TOOL_SCRIPT: &str = include_str!("testdata/fake_tools.cmd");

/// A scratch filesystem + declared environment for deterministic checks.
pub struct TestMachine {
    root: TempDir,
}

impl TestMachine {
    /// A scratch machine with no tools installed.
    pub fn new() -> Self {
        let root = tempfile::tempdir().expect("create test machine root");
        let machine = Self { root };
        fs::create_dir_all(machine.bin()).expect("create fake bin dir");
        fs::create_dir_all(machine.home()).expect("create fake home dir");
        machine
    }

    /// Root of the scratch filesystem; also the host's working directory.
    pub fn root(&self) -> &Path {
        self.root.path()
    }

    /// The directory the host's `PATH` points at.
    pub fn bin(&self) -> PathBuf {
        self.root.path().join("bin")
    }

    /// The host's declared home directory.
    pub fn home(&self) -> PathBuf {
        self.root.path().join("home")
    }

    /// Directory the fake tools read canned responses from.
    pub fn responses(&self) -> PathBuf {
        self.root.path().join("responses")
    }

    /// Install a fake tool on the host's `PATH`; returns its path.
    ///
    /// The fake is a dispatcher script that answers the probes real
    /// toolchain checks perform (`--version`, `sdkmanager --list`,
    /// `adb devices -l`, ...). Canned output comes from env vars
    /// `WATERUI_FAKE_<KEY>` or files under `responses/`.
    pub fn install(&self, name: &str) -> PathBuf {
        Self::script(self.bin().join(tool_file_name(name)))
    }

    /// Write an executable fake tool at `relative` inside the scratch root
    /// (e.g. `sdk/cmdline-tools/latest/bin/sdkmanager`); returns its path.
    pub fn executable(&self, relative: impl AsRef<Path>) -> PathBuf {
        Self::script(self.root.path().join(relative))
    }

    /// Create a plain file inside the scratch root; returns its path.
    pub fn file(&self, relative: impl AsRef<Path>, contents: &str) -> PathBuf {
        let path = self.root.path().join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dirs");
        }
        fs::write(&path, contents).expect("write fixture file");
        path
    }

    /// Create a directory inside the scratch root; returns its path.
    pub fn dir(&self, relative: impl AsRef<Path>) -> PathBuf {
        let path = self.root.path().join(relative);
        fs::create_dir_all(&path).expect("create fixture dir");
        path
    }

    /// Stage a minimal Android SDK under `sdk/` (only `sdkmanager` for now);
    /// returns the SDK root. Detection requires `ANDROID_SDK_ROOT` on the host.
    pub fn install_android_sdk(&self) -> PathBuf {
        let sdk = self.dir("sdk");
        self.executable(
            Path::new("sdk")
                .join("cmdline-tools/latest/bin")
                .join(sdkmanager_file_name()),
        );
        sdk
    }

    /// Stage `platform-tools/adb` inside the scratch SDK.
    ///
    /// On Windows the file is named `adb.exe`, which cannot carry the shell
    /// dispatcher — spawning it fails, which is still a deterministic probe
    /// outcome (the "exists but cannot run" branch).
    pub fn install_adb(&self) -> PathBuf {
        self.executable(
            Path::new("sdk")
                .join("platform-tools")
                .join(adb_file_name()),
        )
    }

    /// Stage `platforms/<api_dir>/android.jar` inside the scratch SDK
    /// (e.g. `android-37` or the minor-versioned `android-37.0`).
    pub fn install_android_platform(&self, api_dir: &str) -> PathBuf {
        self.file(
            Path::new("sdk")
                .join("platforms")
                .join(api_dir)
                .join("android.jar"),
            "",
        )
    }

    /// Stage `build-tools/<version>/lib/d8.jar` inside the scratch SDK.
    pub fn install_android_build_tools(&self, version: &str) -> PathBuf {
        self.file(
            Path::new("sdk")
                .join("build-tools")
                .join(version)
                .join("lib/d8.jar"),
            "",
        )
    }

    /// Stage `ndk/<version>/toolchains/llvm/prebuilt/<tag>/bin/<clang>` inside
    /// the scratch SDK; returns the NDK root.
    ///
    /// The fake clang is the dispatcher script, so the host-toolchain probe
    /// (`clang -x c -c probe.c -o /dev/null`) exits 0 on every platform where
    /// scripts run — on Windows the `.cmd` name gets cmd dispatch.
    pub fn install_android_ndk(&self, version: &str) -> PathBuf {
        self.executable(
            Path::new("sdk")
                .join("ndk")
                .join(version)
                .join("toolchains/llvm/prebuilt/test-host/bin")
                .join(ndk_clang_file_name()),
        );
        self.dir(Path::new("sdk").join("ndk").join(version))
    }

    /// Stage `emulator/emulator` inside the scratch SDK.
    pub fn install_android_emulator(&self) -> PathBuf {
        self.executable(Path::new("sdk").join("emulator").join(emulator_file_name()))
    }

    /// Stage canned output for a fake-tool response key.
    pub fn respond(&self, key: &str, contents: &str) {
        let dir = self.responses();
        fs::create_dir_all(&dir).expect("create responses dir");
        fs::write(dir.join(key), contents).expect("write canned response");
    }

    /// Stage `pkg-config --modversion <module>` output under the response key
    /// the dispatcher derives for that module — the raw module name, so keys
    /// like `PKG_CONFIG_libpipewire-0.3` work identically on every platform
    /// (file lookup only; `-`/`.` cannot appear in a Unix `${}` expansion).
    /// Staging also makes `--exists`/`--atleast-version` pass.
    ///
    /// Only the `#[cfg(target_os = "linux")]` test modules probe pkg-config
    /// modules, so the helper is compiled there only.
    #[cfg(target_os = "linux")]
    pub fn respond_pkg_config_module(&self, module: &str, modversion: &str) {
        self.respond(&format!("PKG_CONFIG_{module}"), modversion);
    }

    /// Stage `pkg-config --variable=<variable> <module>` output.
    #[cfg(target_os = "linux")]
    pub fn respond_pkg_config_var(&self, variable: &str, value: &str) {
        self.respond(&format!("PKG_CONFIG_VAR_{variable}"), value);
    }

    /// The declared host: `PATH` is only `bin/`, `cwd` is the scratch root,
    /// and the environment is exactly `vars` plus `HOME`/`USERPROFILE` and
    /// `WATERUI_FAKE_RESPONSES`.
    pub fn host<K, V>(&self, vars: impl IntoIterator<Item = (K, V)>) -> Host
    where
        K: AsRef<std::ffi::OsStr>,
        V: AsRef<std::ffi::OsStr>,
    {
        let mut declared: Vec<(OsString, OsString)> = vec![
            (
                OsString::from("WATERUI_FAKE_RESPONSES"),
                self.responses().into_os_string(),
            ),
            (OsString::from("HOME"), self.home().into_os_string()),
            (OsString::from("USERPROFILE"), self.home().into_os_string()),
        ];
        declared.extend(
            vars.into_iter()
                .map(|(k, v)| (k.as_ref().to_os_string(), v.as_ref().to_os_string())),
        );
        Host::new([self.bin()], declared).with_cwd(self.root.path().to_path_buf())
    }

    fn script(path: PathBuf) -> PathBuf {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create fake tool dir");
        }
        fs::write(&path, FAKE_TOOL_SCRIPT).expect("write fake tool");
        make_executable(&path);
        path
    }
}

/// Platform file name for a fake tool (`sdkmanager` vs `sdkmanager.cmd`).
///
/// Fake tools on the scratch `PATH` use `.cmd` on Windows; tools embedded in
/// a fake SDK layout (where production code looks for `.bat`/`.exe`) still
/// get the dispatcher content — `cmd` runs `.bat` files the same way — but
/// callers that must match a specific production filename pass the exact
/// name through [`TestMachine::executable`].
pub fn tool_file_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.cmd")
    } else {
        name.to_string()
    }
}

/// The name `sdkmanager` carries inside an SDK layout.
fn sdkmanager_file_name() -> &'static str {
    if cfg!(windows) {
        "sdkmanager.bat"
    } else {
        "sdkmanager"
    }
}

/// The name `adb` carries inside `platform-tools/`.
fn adb_file_name() -> &'static str {
    if cfg!(windows) { "adb.exe" } else { "adb" }
}

/// The name `emulator` carries inside `emulator/`.
fn emulator_file_name() -> &'static str {
    if cfg!(windows) {
        "emulator.exe"
    } else {
        "emulator"
    }
}

/// The API level the staged NDK wrapper carries. The host-toolchain probe
/// accepts any `aarch64-linux-android<api>-clang` wrapper (it picks the lowest
/// level a prebuilt ships), so the fixture stages the lowest level a current
/// NDK provides.
const NDK_WRAPPER_API_LEVEL: u32 = 21;

/// The NDK host clang name the probe looks for
/// (`aarch64-linux-android<api>-clang`).
fn ndk_clang_file_name() -> String {
    let suffix = if cfg!(windows) { ".cmd" } else { "" };
    format!("aarch64-linux-android{NDK_WRAPPER_API_LEVEL}-clang{suffix}")
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
        .expect("mark fake tool executable");
}

#[cfg(windows)]
fn make_executable(_path: &Path) {}
