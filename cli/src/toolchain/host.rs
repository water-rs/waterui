//! The host-machine seam for toolchain probing.
//!
//! Every probe a toolchain check makes against the machine — PATH lookup,
//! environment reads, process spawning — goes through a [`Host`] value instead
//! of process-global state. [`Host::current`] describes the real machine;
//! tests build fully declared hosts with [`Host::new`] so checks run
//! deterministically against fake tools on a private PATH.

use std::{
    collections::BTreeMap,
    env,
    ffi::{OsStr, OsString},
    io,
    path::{Path, PathBuf},
    process::{Output, Stdio},
};

use smol::{io::AsyncReadExt as _, process::Command, unblock};

use crate::utils::{CommandError, format_failure_stream, std_output_enabled};

/// The machine a toolchain check probes.
///
/// A `Host` carries its own environment map (including `PATH`), a working
/// directory for spawned processes and relative-path lookups, a home
/// directory, and the roots that hold installed platform applications
/// (macOS `/Applications`). Detection code must read all of those through
/// this value so a declared host cannot leak real-machine state into a check.
#[derive(Debug, Clone)]
pub struct Host {
    env: BTreeMap<OsString, OsString>,
    cwd: PathBuf,
    home: Option<PathBuf>,
    app_dirs: Vec<PathBuf>,
}

impl Host {
    /// The real machine this process runs on.
    ///
    /// # Panics
    /// Panics when the process has no current directory.
    #[must_use]
    pub fn current() -> Self {
        let env = env::vars_os().collect();
        Self {
            env,
            cwd: env::current_dir().expect("process must have a working directory"),
            home: dirs::home_dir(),
            app_dirs: default_app_dirs(),
        }
    }

    /// A host declared entirely by the arguments.
    ///
    /// `PATH` is exactly `path_dirs`; the environment contains exactly `vars`
    /// plus that `PATH` entry. The working directory defaults to the process
    /// cwd — override it with [`Host::with_cwd`]. The home directory is taken
    /// from the declared `HOME`/`USERPROFILE`; a host that declares neither
    /// has none. Declared hosts have no [`Host::app_dirs`], so application-
    /// bundle fallbacks (e.g. Android Studio's bundled JBR) cannot fire.
    ///
    /// On Windows the process-spawn plumbing variables (`SystemRoot`,
    /// `SystemDrive`, `windir`, `ComSpec`, `PATHEXT`) are seeded from the
    /// running process because children cannot start without them; they
    /// describe how to launch a process, never what is installed.
    ///
    /// # Panics
    /// Panics when `path_dirs` cannot be joined into a `PATH` string (e.g. an
    /// entry containing the platform separator).
    pub fn new<P, K, V>(
        path_dirs: impl IntoIterator<Item = P>,
        vars: impl IntoIterator<Item = (K, V)>,
    ) -> Self
    where
        P: AsRef<Path>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        let mut env = BTreeMap::new();
        seed_process_plumbing(&mut env);
        let path = env::join_paths(
            path_dirs
                .into_iter()
                .map(|dir| dir.as_ref().as_os_str().to_os_string()),
        )
        .expect("Host::new PATH entries must join into a valid PATH string");
        env.insert(OsString::from("PATH"), path);
        for (key, value) in vars {
            env.insert(key.as_ref().to_os_string(), value.as_ref().to_os_string());
        }
        let home = home_dir_from_env(&env);
        Self {
            env,
            cwd: env::current_dir().expect("process must have a working directory"),
            home,
            app_dirs: Vec::new(),
        }
    }

    /// Override the working directory (builder-style).
    #[must_use]
    pub fn with_cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = cwd.into();
        self
    }

    /// Override the application-install roots (builder-style).
    #[must_use]
    pub fn with_app_dirs(mut self, app_dirs: impl IntoIterator<Item = PathBuf>) -> Self {
        self.app_dirs = app_dirs.into_iter().collect();
        self
    }

    /// An environment variable on this host.
    ///
    /// Lookup is case-sensitive on Unix and case-insensitive on Windows,
    /// matching the platform's own environment semantics.
    #[must_use]
    pub fn env(&self, key: impl AsRef<OsStr>) -> Option<&OsStr> {
        env_get(&self.env, key.as_ref())
    }

    /// An environment variable decoded as UTF-8 text.
    #[must_use]
    pub fn env_string(&self, key: impl AsRef<OsStr>) -> Option<String> {
        self.env(key)
            .and_then(|value| value.to_str().map(ToOwned::to_owned))
    }

    /// This host's `PATH` entries, in order.
    ///
    /// Empty components are dropped: an empty `PATH` element historically
    /// means "the working directory", which would let tools sitting in
    /// [`Host::cwd`] masquerade as installed on a declared host.
    #[must_use]
    pub fn path_entries(&self) -> Vec<PathBuf> {
        self.env("PATH")
            .map(|paths| {
                env::split_paths(paths)
                    .filter(|entry| !entry.as_os_str().is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Working directory for spawned processes and relative-path lookups.
    #[must_use]
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// This host's home directory, when it declares one.
    #[must_use]
    pub fn home_dir(&self) -> Option<&Path> {
        self.home.as_deref()
    }

    /// Roots holding installed platform application bundles.
    ///
    /// `/Applications` on macOS, empty elsewhere and on declared hosts.
    /// Probes that look inside installed `.app` bundles read them under these
    /// roots so test machines do not leak real-machine installs.
    #[must_use]
    pub fn app_dirs(&self) -> &[PathBuf] {
        &self.app_dirs
    }

    /// Locate `name` on this host's `PATH`.
    ///
    /// Never consults the process `PATH`: a host with no `PATH` or an empty
    /// one reports every tool as missing.
    ///
    /// # Errors
    /// - [`which::Error`] when no executable named `name` exists on this host.
    pub async fn which(&self, name: impl AsRef<OsStr>) -> Result<PathBuf, which::Error> {
        let name = name.as_ref().to_os_string();
        let paths = self.joined_path();
        let cwd = self.cwd.clone();
        unblock(move || which::which_in(name, paths, cwd)).await
    }

    /// A host whose environment additionally binds `key` to `value`.
    ///
    /// Use for variables that must reach a single child tree (for example
    /// `WATERUI_SKIP_RUST_BUILD` on the `xcodebuild` invocation) instead of
    /// mutating the process environment.
    #[must_use]
    pub fn with_env(&self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        let mut host = self.clone();
        host.env
            .insert(key.as_ref().to_os_string(), value.as_ref().to_os_string());
        host
    }

    /// A [`Command`] that runs `program` under this host's environment.
    ///
    /// The child sees exactly this host's variables and starts in
    /// [`Host::cwd`]; `program` is resolved against this host's `PATH`.
    /// stdio configuration is left to the caller — see [`crate::utils::command`]
    /// for the CLI's capture/inherit policy.
    #[must_use]
    pub fn command(&self, program: impl AsRef<OsStr>) -> Command {
        let mut command = Command::new(self.resolve_program(program.as_ref()));
        command.env_clear().envs(&self.env).current_dir(&self.cwd);
        command
    }

    /// A [`std::process::Command`] that runs `program` under this host.
    ///
    /// Same environment and working directory as [`Host::command`], for the
    /// places that need synchronous or `std`-only command features (process
    /// groups, spawning from a non-async thread).
    #[must_use]
    pub fn std_command(&self, program: impl AsRef<OsStr>) -> std::process::Command {
        let mut command = std::process::Command::new(self.resolve_program(program.as_ref()));
        command.env_clear().envs(&self.env).current_dir(&self.cwd);
        command
    }

    /// Spawn `program` with `args` under this host, capturing output.
    ///
    /// stdout and stderr are piped and always collected for the returned
    /// [`Output`]; when the CLI's `--logs` passthrough is active each chunk is
    /// additionally mirrored to the terminal as it arrives, matching the
    /// historical `run_command_output_os` behavior.
    ///
    /// # Errors
    /// - [`CommandError::Spawn`] when the program cannot be spawned or awaited.
    ///
    /// # Panics
    /// Panics if the piped-stdio invariant above is violated — both streams
    /// are configured `piped` immediately before spawn, so `take()` always
    /// sees `Some`.
    pub async fn output(
        &self,
        program: impl AsRef<OsStr>,
        args: impl IntoIterator<Item = impl AsRef<OsStr>>,
    ) -> Result<Output, CommandError> {
        let program = program.as_ref();
        let program_name = program.to_string_lossy().into_owned();
        let mut command = self.command(program);
        command
            .args(args)
            .kill_on_drop(true)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().map_err(|source| CommandError::Spawn {
            program: program_name.clone(),
            source,
        })?;

        let echo = std_output_enabled();
        let stdout_task = smol::spawn(drain_child_pipe(
            child.stdout.take().expect("stdout is piped"),
            io::stdout(),
            echo,
        ));
        let stderr_task = smol::spawn(drain_child_pipe(
            child.stderr.take().expect("stderr is piped"),
            io::stderr(),
            echo,
        ));

        let status = child.status().await.map_err(|source| CommandError::Spawn {
            program: program_name.clone(),
            source,
        })?;
        let stdout = stdout_task.await.map_err(|source| CommandError::Spawn {
            program: program_name.clone(),
            source,
        })?;
        let stderr = stderr_task.await.map_err(|source| CommandError::Spawn {
            program: program_name,
            source,
        })?;
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    }

    /// Run `program` under this host and return stdout as text.
    ///
    /// # Errors
    /// - [`CommandError::Spawn`] when the program cannot be spawned.
    /// - [`CommandError::Failed`] when it exits non-zero; the error embeds
    ///   the captured stderr/stdout tails.
    pub async fn run(
        &self,
        program: impl AsRef<OsStr>,
        args: impl IntoIterator<Item = impl AsRef<OsStr>>,
    ) -> Result<String, CommandError> {
        let program = program.as_ref();
        let output = self.output(program, args).await?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(CommandError::Failed {
                program: program.to_string_lossy().into_owned(),
                status: output.status,
                report: format!(
                    "{}{}",
                    format_failure_stream("stderr", &output.stderr),
                    format_failure_stream("stdout", &output.stdout),
                ),
            })
        }
    }

    /// Resolve a bare program name against this host's `PATH`.
    ///
    /// `CreateProcess` searches the *parent's* `PATH`, never the child's, so
    /// passing a bare name through `env_clear` + `envs` would miss tools that
    /// exist only on this host — exactly what fake-tool tests install.
    /// Resolving here makes `command`/`std_command`/`output` agree with
    /// [`Host::which`] on every platform. Paths (anything with a separator)
    /// and names this host cannot resolve pass through unchanged, so a spawn
    /// error still names what the caller asked for.
    fn resolve_program(&self, program: &OsStr) -> OsString {
        let path = Path::new(program);
        if path.components().count() > 1 {
            return program.to_os_string();
        }
        let paths = self.joined_path();
        which::which_in(program, paths, &self.cwd)
            .map_or_else(|_| program.to_os_string(), PathBuf::into_os_string)
    }

    /// The declared `PATH` re-joined after empty-component filtering.
    ///
    /// `None` when the host declares no usable `PATH`, which makes
    /// `which::which_in` report every lookup as missing instead of
    /// searching the working directory.
    fn joined_path(&self) -> Option<OsString> {
        let entries = self.path_entries();
        if entries.is_empty() {
            return None;
        }
        Some(
            env::join_paths(entries)
                .expect("PATH entries produced by split_paths re-join into a PATH string"),
        )
    }
}

/// Drain a piped child stream to EOF.
///
/// Every chunk is appended to the returned buffer; when `echo` is set it is
/// also written to `sink` (the matching terminal stream) as it arrives, so
/// `--logs` output appears incrementally instead of after the process exits.
/// Terminal write failures are ignored — a broken sink must not kill output
/// collection.
async fn drain_child_pipe(
    mut reader: impl smol::io::AsyncRead + Unpin,
    mut sink: impl io::Write,
    echo: bool,
) -> io::Result<Vec<u8>> {
    let mut collected = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let read = reader.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        if echo {
            let _ = sink.write_all(&chunk[..read]);
            let _ = sink.flush();
        }
        collected.extend_from_slice(&chunk[..read]);
    }
    Ok(collected)
}

/// Case-aware environment lookup matching platform semantics.
fn env_get<'a>(env: &'a BTreeMap<OsString, OsString>, key: &OsStr) -> Option<&'a OsStr> {
    if cfg!(target_os = "windows") {
        env.iter()
            .find(|(existing, _)| existing.as_os_str().eq_ignore_ascii_case(key))
            .map(|(_, value)| value.as_os_str())
    } else {
        env.get(key).map(OsString::as_os_str)
    }
}

/// Home directory derived purely from a declared environment map.
fn home_dir_from_env(env: &BTreeMap<OsString, OsString>) -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        env_get(env, "USERPROFILE".as_ref())
            .or_else(|| env_get(env, "HOME".as_ref()))
            .map(PathBuf::from)
    } else {
        env_get(env, "HOME".as_ref())
            .or_else(|| env_get(env, "USERPROFILE".as_ref()))
            .map(PathBuf::from)
    }
}

/// Application-install roots for the real machine.
fn default_app_dirs() -> Vec<PathBuf> {
    if cfg!(target_os = "macos") {
        vec![PathBuf::from("/Applications")]
    } else {
        Vec::new()
    }
}

/// Seed variables a spawned Windows child cannot start without.
#[cfg(target_os = "windows")]
fn seed_process_plumbing(env: &mut BTreeMap<OsString, OsString>) {
    for key in ["SystemRoot", "SystemDrive", "windir", "ComSpec", "PATHEXT"] {
        if let Some(value) = env::var_os(key) {
            env.entry(OsString::from(key)).or_insert(value);
        }
    }
}

#[cfg(not(target_os = "windows"))]
const fn seed_process_plumbing(_env: &mut BTreeMap<OsString, OsString>) {}

#[cfg(test)]
mod tests {
    use super::Host;
    use crate::toolchain::testing::TestMachine;

    /// A variable name nothing declares — proves reads never reach the
    /// ambient process environment.
    const UNDECLARED: &str = "WATERUI_TEST_NEVER_DECLARED";

    #[test]
    fn declared_host_env_contains_only_what_was_declared() {
        let host = Host::new(
            Vec::<std::path::PathBuf>::new(),
            [(String::from("WATERUI_TEST_DECLARED"), String::from("yes"))],
        );
        assert_eq!(
            host.env_string("WATERUI_TEST_DECLARED").as_deref(),
            Some("yes")
        );
        assert!(
            host.env(UNDECLARED).is_none(),
            "declared hosts must not see ambient environment variables"
        );
        // PATH is exactly what was declared — here, nothing.
        assert!(host.path_entries().is_empty());
    }

    #[test]
    fn declared_host_home_comes_from_declared_env() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        assert_eq!(host.home_dir(), Some(machine.home().as_path()));
        assert_eq!(host.cwd(), machine.root());
        assert!(
            host.app_dirs().is_empty(),
            "declared hosts never see installed application bundles"
        );
    }

    #[test]
    fn which_resolves_only_the_host_path() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(async {
            assert!(host.which("waterui-test-missing-tool").await.is_err());
            assert!(
                host.which("cargo").await.is_err(),
                "real cargo must not leak"
            );
            machine.install("cargo");
            let resolved = host
                .which("cargo")
                .await
                .expect("installed fake tool must resolve");
            assert_eq!(resolved.parent(), Some(machine.bin().as_path()));
        });
    }

    #[test]
    fn spawned_children_see_the_declared_environment() {
        let machine = TestMachine::new();
        machine.install("cargo");
        let host = machine.host([(
            String::from("WATERUI_FAKE_CARGO_VERSION"),
            String::from("9.9.9-waterui-test"),
        )]);
        let output = smol::block_on(host.run("cargo", ["--version"]))
            .expect("fake cargo must run under the declared host");
        assert!(output.contains("9.9.9-waterui-test"));
    }

    #[test]
    fn run_reports_nonzero_exit_with_output() {
        let machine = TestMachine::new();
        machine.install("rustup");
        let host = machine.host(Vec::<(String, String)>::new());
        // `rustup frobnicate` is not a dispatched case → exit 2.
        let error = smol::block_on(host.run("rustup", ["frobnicate"]))
            .expect_err("a failing tool must surface as an error");
        assert!(error.to_string().contains("rustup"));
    }
}
