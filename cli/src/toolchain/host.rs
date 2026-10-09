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

use cargo_metadata::{Metadata, MetadataCommand};
use smol::{io::AsyncReadExt as _, process::Command, unblock};

use crate::utils::{CommandError, format_failure_stream, std_output_enabled};

mod detached;

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

    /// Per-user cache directory for this host.
    ///
    /// `dirs::cache_dir` semantics read through the host's environment, so
    /// the CLI and a build script that asks `dirs` agree on the path:
    /// `~/Library/Caches` on macOS; on other Unix systems `XDG_CACHE_HOME`
    /// when it is an absolute path, else `$HOME/.cache`; on Windows the
    /// local application-data folder, which this host reads from its
    /// `LOCALAPPDATA` (`dirs` asks the known-folder API, which reports the
    /// same folder). `None` when the host declares no usable value.
    #[must_use]
    pub fn cache_dir(&self) -> Option<PathBuf> {
        #[cfg(target_os = "windows")]
        {
            self.absolute_env_path("LOCALAPPDATA")
        }
        #[cfg(target_os = "macos")]
        {
            self.home_dir().map(|home| home.join("Library/Caches"))
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            // `dirs` ignores an empty or relative `XDG_CACHE_HOME`, as the
            // XDG base-directory specification requires.
            self.absolute_env_path("XDG_CACHE_HOME")
                .or_else(|| self.home_dir().map(|home| home.join(".cache")))
        }
        #[cfg(not(any(unix, target_os = "windows")))]
        {
            None
        }
    }

    /// Temporary directory for this host.
    ///
    /// `std::env::temp_dir` rules, with every variable read from this host's
    /// environment and never the process's:
    /// - Unix: `TMPDIR` when set; otherwise the per-user directory
    ///   `confstr(_CS_DARWIN_USER_TEMP_DIR)` reports on Apple platforms
    ///   (`/tmp` if it reports none), `/data/local/tmp` on Android, and
    ///   `/tmp` elsewhere.
    /// - Windows (`GetTempPath2W`): the first of `TMP`, `TEMP` and
    ///   `USERPROFILE` that is set and non-empty, else the Windows directory
    ///   (`SystemRoot`, which every host carries as spawn plumbing).
    ///
    /// # Panics
    /// On Windows, panics when the host declares none of those variables and
    /// no `SystemRoot` either — a host no child process could start on.
    #[must_use]
    pub fn temp_dir(&self) -> PathBuf {
        #[cfg(unix)]
        {
            self.env("TMPDIR")
                .map_or_else(unix_default_temp_dir, PathBuf::from)
        }
        #[cfg(windows)]
        {
            ["TMP", "TEMP", "USERPROFILE", "SystemRoot"]
                .into_iter()
                .find_map(|key| self.env(key).filter(|value| !value.is_empty()))
                .map(PathBuf::from)
                .expect("a Windows host carries SystemRoot, the temp directory of last resort")
        }
    }

    /// `key` as a path, when this host sets it to an absolute one.
    #[cfg(not(target_os = "macos"))]
    fn absolute_env_path(&self, key: &str) -> Option<PathBuf> {
        self.env(key)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    }

    /// Path of the running `water` executable.
    ///
    /// A fact about this process rather than the declared machine — every
    /// `Host` reports the same binary, which is what `RUSTC_WRAPPER`
    /// self-wrapping must name. Associated with [`Host`] so the probe stays
    /// inside the seam.
    ///
    /// # Errors
    /// Returns an error when the OS cannot report the executable's path.
    pub fn current_exe() -> io::Result<PathBuf> {
        env::current_exe()
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

    /// [`Host::which`] on the calling thread, for code that is synchronous
    /// end to end (such as classifying the running executable's install).
    /// Async code calls [`Host::which`], which moves the filesystem probes
    /// off the executor.
    ///
    /// # Errors
    /// - [`which::Error`] when no executable named `name` exists on this host.
    pub fn which_blocking(&self, name: impl AsRef<OsStr>) -> Result<PathBuf, which::Error> {
        which::which_in(name, self.joined_path(), &self.cwd)
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

    /// Every environment variable on this host, in map order.
    ///
    /// For resolvers that take the whole environment at once rather than
    /// probing one variable at a time — [`cargo_config2::ResolveOptions::env`]
    /// being the one in-tree caller.
    pub fn envs(&self) -> impl Iterator<Item = (&OsStr, &OsStr)> {
        self.env.iter().map(|(k, v)| (k.as_os_str(), v.as_os_str()))
    }

    /// A [`Command`] that runs `program` under this host's environment.
    ///
    /// The child sees exactly this host's variables and starts in
    /// [`Host::cwd`]; `program` is resolved against this host's `PATH`.
    /// stdin is `null`: a child never holds this process's own stdin, which
    /// under `water mcp` is the agent host's JSON-RPC pipe — a caller that
    /// feeds or forwards input sets `.stdin(...)` on the returned command
    /// explicitly. Other stdio is left to the caller — see
    /// `crate::utils::command` for the CLI's capture/inherit policy.
    #[must_use]
    pub fn command(&self, program: impl AsRef<OsStr>) -> Command {
        withhold_std_handles_from_children();
        let mut command = Command::new(self.resolve_program(program.as_ref()));
        command
            .env_clear()
            .envs(&self.env)
            .current_dir(&self.cwd)
            .stdin(Stdio::null());
        command
    }

    /// A [`std::process::Command`] that runs `program` under this host.
    ///
    /// Same environment, working directory and null-stdin default as
    /// [`Host::command`], for the places that need synchronous or `std`-only
    /// command features (process groups, spawning from a non-async thread).
    #[must_use]
    pub fn std_command(&self, program: impl AsRef<OsStr>) -> std::process::Command {
        withhold_std_handles_from_children();
        let mut command = std::process::Command::new(self.resolve_program(program.as_ref()));
        command
            .env_clear()
            .envs(&self.env)
            .current_dir(&self.cwd)
            .stdin(Stdio::null());
        command
    }

    /// A [`std::process::Command`] that runs `program` under this host with
    /// the invoking terminal attached: stdin, stdout and stderr all
    /// inherited.
    ///
    /// This is the deliberate exception to the null-stdin default of
    /// [`Host::command`] — for the tools that interact with the user's
    /// terminal: the `create vite` framework picker, `<pm> install`, and
    /// launchers that take the TTY over entirely. The
    /// `std` type is returned so callers that `exec` or group the child can;
    /// async callers wrap it with `smol::process::Command::from`.
    #[must_use]
    pub fn interactive_command(&self, program: impl AsRef<OsStr>) -> std::process::Command {
        let mut command = self.std_command(program);
        command
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        command
    }

    /// A [`std::process::Command`] that inherits the terminal for output but
    /// reads nothing: stdin keeps the null default of
    /// [`Host::std_command`].
    ///
    /// For a tool whose progress and errors the user watches live but that
    /// never reads stdin — `water bench` running `cargo nextest` — in
    /// contrast to [`Host::interactive_command`], which owns the whole
    /// terminal.
    #[must_use]
    pub fn monitored_command(&self, program: impl AsRef<OsStr>) -> std::process::Command {
        let mut command = self.std_command(program);
        command.stdout(Stdio::inherit()).stderr(Stdio::inherit());
        command
    }

    /// A [`Command`] that spawns `program` as the leader of a new process
    /// group.
    ///
    /// Same environment and working directory as [`Host::command`]. A child
    /// in its own process group cannot receive the `SIGINT` a terminal sends
    /// its foreground group: `Ctrl-C` reaches this process alone, and the
    /// parent forwards one termination signal itself. Sharing a group would
    /// let the terminal signal the child directly — a second, unsupervised
    /// termination path the supervisor cannot order or count. The group
    /// boundary exists only on Unix, where `std` lowers `process_group(0)`
    /// to `setpgid` in the child before `exec`.
    #[cfg(unix)]
    #[must_use]
    pub fn command_in_own_process_group(&self, program: impl AsRef<OsStr>) -> Command {
        use std::os::unix::process::CommandExt as _;

        let mut command = self.std_command(program);
        command.process_group(0);
        Command::from(command)
    }

    /// Spawn `program` with `args` under this host, capturing output.
    ///
    /// stdin is null (see [`Host::command`]); stdout and stderr are piped and
    /// always collected for the returned [`Output`]; when the CLI's `--logs` passthrough is
    /// active each chunk is additionally mirrored to the terminal as it
    /// arrives, matching the historical `run_command_output_os` behavior.
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
        let args = args
            .into_iter()
            .map(|argument| argument.as_ref().to_os_string())
            .collect::<Vec<_>>();
        tracing::debug!(program = %program_name, ?args, "spawning");
        let started = std::time::Instant::now();
        let mut command = self.command(program);
        command
            .args(&args)
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
            program: program_name.clone(),
            source,
        })?;
        tracing::debug!(
            program = %program_name,
            %status,
            elapsed_ms = started.elapsed().as_millis(),
            "exited"
        );
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

    /// `cargo metadata` as `command` configures it, run on this host.
    ///
    /// [`MetadataCommand::exec`] spawns cargo with the process's own
    /// environment. This takes the invocation `command` describes — its
    /// arguments, working directory and environment overrides — and runs it
    /// through [`Host::command`] instead, so cargo is found on this host's
    /// `PATH` and sees this host's environment. The output contract is
    /// `exec`'s: a failed run is [`cargo_metadata::Error::CargoMetadata`]
    /// carrying cargo's stderr, and the metadata is the first stdout line
    /// that opens a JSON object, parsed on the blocking pool.
    ///
    /// # Errors
    /// Every error [`MetadataCommand::exec`] reports.
    pub async fn cargo_metadata(
        &self,
        command: &MetadataCommand,
    ) -> Result<Metadata, cargo_metadata::Error> {
        let invocation = command.cargo_command();
        let mut cargo = self.command("cargo");
        cargo.args(invocation.get_args()).kill_on_drop(true);
        if let Some(dir) = invocation.get_current_dir() {
            cargo.current_dir(dir);
        }
        for (key, value) in invocation.get_envs() {
            match value {
                Some(value) => cargo.env(key, value),
                None => cargo.env_remove(key),
            };
        }
        let output = cargo.output().await?;
        unblock(move || {
            if !output.status.success() {
                return Err(cargo_metadata::Error::CargoMetadata {
                    stderr: String::from_utf8(output.stderr)?,
                });
            }
            let json = std::str::from_utf8(&output.stdout)?
                .lines()
                .find(|line| line.starts_with('{'))
                .ok_or(cargo_metadata::Error::NoJson)?;
            MetadataCommand::parse(json)
        })
        .await
    }

    /// Run `program` to completion with nothing of this process in its hands:
    /// no stdio and, on Windows, no inherited handles at all.
    ///
    /// This is how a daemon launcher is run. A child spawned the ordinary way
    /// receives every inheritable handle this process holds — including
    /// strays our own parent passed down — and hands them on to whatever it
    /// spawns with inheritance on. `adb start-server` is the case that
    /// matters: its server outlives `water`, and a pipe it inherited stays
    /// open until the server exits. The exit status is the caller's to judge,
    /// because a launcher's own output is discarded here.
    ///
    /// # Errors
    /// [`CommandError::Spawn`] when the program cannot be started or waited
    /// on.
    pub async fn run_detached(
        &self,
        program: impl AsRef<OsStr>,
        args: impl IntoIterator<Item = impl AsRef<OsStr>>,
    ) -> Result<std::process::ExitStatus, CommandError> {
        let program_name = program.as_ref().to_string_lossy().into_owned();
        let args = args
            .into_iter()
            .map(|argument| argument.as_ref().to_os_string())
            .collect::<Vec<_>>();
        tracing::debug!(program = %program_name, ?args, "spawning detached");
        let resolved = self.resolve_program(program.as_ref());
        let env = self.env.clone();
        let cwd = self.cwd.clone();
        let status = unblock(move || detached::run(&resolved, &args, &env, &cwd))
            .await
            .map_err(|source| CommandError::Spawn {
                program: program_name.clone(),
                source,
            })?;
        tracing::debug!(program = %program_name, %status, "detached launcher exited");
        Ok(status)
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

/// A child of this process must hold only the stdio it is given, never this
/// process's own standard handles.
///
/// `CreateProcess` hands a child every inheritable handle of its parent, and
/// the standard handles a shell passes in arrive inheritable, so a child
/// spawned with piped stdio still receives this process's stdout and stderr
/// as stray handles — and so does anything the child spawns with inheritance
/// on. `adb` is the case that bites: its first client command launches the
/// server daemon, which then outlives `water` holding the pipe whoever ran
/// `water` is reading, and that reader never sees end-of-file. Clearing the
/// inherit flag on our own standard handles ends the chain at the source;
/// `Stdio::inherit` still works, because the standard library duplicates the
/// handle inheritably for the one child that is meant to have it.
#[cfg(windows)]
fn withhold_std_handles_from_children() {
    use windows_sys::Win32::{
        Foundation::{HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, SetHandleInformation},
        System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE},
    };

    for (name, id) in [
        ("stdin", STD_INPUT_HANDLE),
        ("stdout", STD_OUTPUT_HANDLE),
        ("stderr", STD_ERROR_HANDLE),
    ] {
        // SAFETY: querying this process's own standard handle table.
        let handle = unsafe { GetStdHandle(id) };
        // A process started without that stream has nothing to withhold.
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            continue;
        }
        // SAFETY: `handle` is a live handle of this process; clearing its
        // inherit flag changes nothing about how this process uses it.
        let cleared = unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) };
        assert!(
            cleared != 0,
            "failed to make {name} non-inheritable: {}",
            io::Error::last_os_error()
        );
    }
}

#[cfg(not(windows))]
const fn withhold_std_handles_from_children() {
    // POSIX children receive only the descriptors we pass: every descriptor
    // the standard library opens is close-on-exec, and daemons detach through
    // fork rather than handle inheritance.
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

/// The temp directory `std` uses on Unix when `TMPDIR` is unset.
#[cfg(unix)]
fn unix_default_temp_dir() -> PathBuf {
    #[cfg(target_vendor = "apple")]
    {
        darwin_user_temp_dir().unwrap_or_else(|| PathBuf::from("/tmp"))
    }
    #[cfg(target_os = "android")]
    {
        PathBuf::from("/data/local/tmp")
    }
    #[cfg(not(any(target_vendor = "apple", target_os = "android")))]
    {
        PathBuf::from("/tmp")
    }
}

/// The per-user temp directory Darwin reports through
/// `confstr(_CS_DARWIN_USER_TEMP_DIR)`, or `None` when it reports none.
#[cfg(target_vendor = "apple")]
fn darwin_user_temp_dir() -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt as _;

    let mut buffer = vec![0_u8; 64];
    loop {
        // SAFETY: `buffer` is valid for writes of `buffer.len()` bytes, the
        // length `confstr` is told it may fill.
        let needed = unsafe {
            libc::confstr(
                libc::_CS_DARWIN_USER_TEMP_DIR,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
            )
        };
        if needed == 0 {
            return None;
        }
        if needed <= buffer.len() {
            // `needed` counts the terminating NUL.
            buffer.truncate(needed - 1);
            return Some(PathBuf::from(OsString::from_vec(buffer)));
        }
        buffer.resize(needed, 0);
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
        assert_eq!(host.path_entries(), [] as [std::path::PathBuf; 0]);
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

    /// A child spawned through [`Host::command_in_own_process_group`] leads
    /// its own process group — `getpgid` answers the child's pid — so a
    /// terminal `SIGINT` addressed to this process's group cannot reach it.
    #[cfg(unix)]
    #[test]
    fn spawned_child_leads_its_own_process_group() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(async {
            let mut child = host
                .command_in_own_process_group("/bin/sh")
                .arg("-c")
                .arg("/bin/sleep 60")
                .kill_on_drop(true)
                .spawn()
                .expect("spawn a child through the process-group seam");
            let pid = nix::unistd::Pid::from_raw(
                i32::try_from(child.id()).expect("a child's pid fits in i32"),
            );
            let group = nix::unistd::getpgid(Some(pid)).expect("read the child's group");
            assert_eq!(group, pid, "the child must lead its own process group");
            assert_ne!(
                group,
                nix::unistd::getpgrp(),
                "the child's group must differ from the supervisor's"
            );
            let _ = child.kill();
            let _ = child.status().await;
        });
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

    fn declared(vars: &[(&str, &str)]) -> Host {
        Host::new(Vec::<std::path::PathBuf>::new(), vars.iter().copied())
    }

    #[cfg(unix)]
    #[test]
    fn temp_dir_is_the_declared_tmpdir() {
        assert_eq!(
            declared(&[("TMPDIR", "/declared/tmp")]).temp_dir(),
            std::path::Path::new("/declared/tmp")
        );
    }

    #[cfg(all(unix, not(target_vendor = "apple"), not(target_os = "android")))]
    #[test]
    fn temp_dir_without_tmpdir_is_slash_tmp() {
        assert_eq!(declared(&[]).temp_dir(), std::path::Path::new("/tmp"));
    }

    #[cfg(target_vendor = "apple")]
    #[test]
    fn temp_dir_without_tmpdir_is_the_darwin_user_temp_dir() {
        let temp = declared(&[]).temp_dir();
        assert!(temp.is_absolute(), "{}", temp.display());
        assert!(temp.is_dir(), "{} must exist", temp.display());
    }

    #[cfg(windows)]
    #[test]
    fn temp_dir_follows_get_temp_path_order() {
        let all = [
            ("TMP", r"C:\tmp-var"),
            ("TEMP", r"C:\temp-var"),
            ("USERPROFILE", r"C:\Users\declared"),
            ("SystemRoot", r"C:\Windows-declared"),
        ];
        for skipped in 0..all.len() {
            let host = declared(&all[skipped..]);
            assert_eq!(host.temp_dir(), std::path::Path::new(all[skipped].1));
        }
        assert_eq!(
            declared(&[("TMP", ""), ("TEMP", r"C:\temp-var")]).temp_dir(),
            std::path::Path::new(r"C:\temp-var"),
            "an empty variable is skipped"
        );
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn cache_dir_ignores_a_relative_or_empty_xdg_cache_home() {
        for value in ["", "relative/cache"] {
            assert_eq!(
                declared(&[("HOME", "/home/declared"), ("XDG_CACHE_HOME", value)]).cache_dir(),
                Some(std::path::PathBuf::from("/home/declared/.cache")),
                "XDG_CACHE_HOME={value:?}"
            );
        }
        assert_eq!(
            declared(&[("HOME", "/home/declared"), ("XDG_CACHE_HOME", "/xdg/cache")]).cache_dir(),
            Some(std::path::PathBuf::from("/xdg/cache"))
        );
    }
}
