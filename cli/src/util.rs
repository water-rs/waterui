use std::{
    env,
    path::Path,
    process::{Child, Command, ExitStatus},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
};

use color_eyre::eyre::{Context, Result, bail};
use tracing::warn;
use which::which;

/// Global flag to track if we've received a termination signal.
/// This is shared across all child process runs.
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// Check if the process has been interrupted by a signal.
#[inline]
pub fn is_interrupted() -> bool {
    INTERRUPTED.load(Ordering::Relaxed)
}

/// Mark the process as interrupted (called from signal handlers).
pub fn set_interrupted() {
    INTERRUPTED.store(true, Ordering::Relaxed);
}

/// Reset the interrupted flag (call at start of commands).
pub fn reset_interrupted() {
    INTERRUPTED.store(false, Ordering::Relaxed);
}

/// Run a command with proper signal handling.
///
/// Unlike `.status()` which blocks until completion, this spawns the child
/// and polls for completion, checking for interrupt signals. When interrupted,
/// it kills the child process and returns an error.
///
/// This solves the "double Ctrl+C" problem where the first Ctrl+C goes to the
/// child process but the parent is blocked in `.status()`.
pub fn run_command_interruptible(mut cmd: Command) -> Result<ExitStatus> {
    let mut child = cmd.spawn().context("failed to spawn command")?;
    wait_for_child_interruptible(&mut child)
}

/// Wait for a child process with interrupt handling.
///
/// Polls the child for completion every 50ms, checking for interrupt signals.
/// If interrupted, kills the child and returns an error.
pub fn wait_for_child_interruptible(child: &mut Child) -> Result<ExitStatus> {
    loop {
        // Check for interrupt signal
        if is_interrupted() {
            // Kill the child process
            let _ = child.kill();
            let _ = child.wait(); // Reap the zombie
            bail!("Build interrupted by user");
        }

        // Check if child has exited
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {
                // Child still running, sleep briefly before next check
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                return Err(e).context("failed to wait for child process");
            }
        }
    }
}

/// Ensure a directory exists, creating missing parents when needed.
///
/// # Errors
/// Returns an error if the directory cannot be created.
pub fn ensure_directory(path: &Path) -> Result<()> {
    if !path.exists() {
        std::fs::create_dir_all(path)?;
    }
    Ok(())
}

/// Inject standard environment variables that toggle `WaterUI` hot reload at runtime.
/// Also sets the `--cfg waterui_hot_reload_lib` compile-time flag via RUSTFLAGS.
///
/// The `waterui_hot_reload_lib` flag indicates that the compiled dylib can be dynamically
/// loaded for hot reload. The app internally switches the entry point from `waterui_main`
/// to `waterui_hot_reload_main` to prevent infinite loading loops.
pub fn configure_hot_reload_env(cmd: &mut Command, enable: bool, port: Option<u16>) {
    const HOT_RELOAD_CFG: &str = "--cfg=waterui_hot_reload_lib";

    if enable {
        cmd.env("WATERUI_ENABLE_HOT_RELOAD", "1");
        cmd.env("WATERUI_HOT_RELOAD_HOST", "127.0.0.1");
        if let Some(port) = port {
            cmd.env("WATERUI_HOT_RELOAD_PORT", port.to_string());
        }

        // Set compile-time cfg flag
        let mut rustflags: Vec<String> = env::var("RUSTFLAGS")
            .ok()
            .map(|flags| flags.split_whitespace().map(ToString::to_string).collect())
            .unwrap_or_default();

        if !rustflags.iter().any(|f| f == HOT_RELOAD_CFG) {
            rustflags.push(HOT_RELOAD_CFG.to_string());
            cmd.env("RUSTFLAGS", rustflags.join(" "));
        }
    } else {
        cmd.env("WATERUI_ENABLE_HOT_RELOAD", "1");
        cmd.env_remove("WATERUI_HOT_RELOAD_HOST");
        cmd.env_remove("WATERUI_HOT_RELOAD_PORT");

        // Remove hot reload cfg flag to avoid wrapping views in Hotreload
        // This is important for incremental hot reload builds
        let rustflags: Vec<String> = env::var("RUSTFLAGS")
            .ok()
            .map(|flags| {
                flags
                    .split_whitespace()
                    .filter(|f| *f != HOT_RELOAD_CFG)
                    .map(ToString::to_string)
                    .collect()
            })
            .unwrap_or_default();
        cmd.env("RUSTFLAGS", rustflags.join(" "));
    }
}

/// Apply build speedups (sccache, mold) to a Cargo command.
///
/// Returns `true` if sccache was enabled so callers can retry without it when builds fail.
pub fn configure_build_speedups(
    cmd: &mut Command,
    enable_sccache: bool,
    enable_mold: bool,
) -> bool {
    let sccache_enabled = if enable_sccache {
        configure_sccache(cmd)
    } else {
        false
    };
    configure_mold(cmd, enable_mold);
    sccache_enabled
}

fn configure_sccache(cmd: &mut Command) -> bool {
    if env::var_os("RUSTC_WRAPPER").is_some() {
        warn!("RUSTC_WRAPPER already set; not overriding with sccache");
        return false;
    }

    which("sccache").map_or_else(
        |_| {
            warn!("`sccache` not found on PATH; proceeding without build cache");
            false
        },
        |path| {
            cmd.env("RUSTC_WRAPPER", path);
            true
        },
    )
}

#[cfg(target_os = "linux")]
fn configure_mold(cmd: &mut Command, enable: bool) {
    if !enable {
        return;
    }

    const MOLD_FLAG: &str = "-C";
    const MOLD_VALUE: &str = "link-arg=-fuse-ld=mold";

    let mut rustflags: Vec<String> = env::var("RUSTFLAGS")
        .ok()
        .map(|flags| flags.split_whitespace().map(ToString::to_string).collect())
        .unwrap_or_default();

    let already_set = rustflags
        .windows(2)
        .any(|win| win == [MOLD_FLAG, MOLD_VALUE]);

    if already_set {
        return;
    }

    rustflags.push(MOLD_FLAG.to_string());
    rustflags.push(MOLD_VALUE.to_string());
    let joined = rustflags.join(" ");
    cmd.env("RUSTFLAGS", joined);
}

#[cfg(not(target_os = "linux"))]
const fn configure_mold(_cmd: &mut Command, _enable: bool) {}
