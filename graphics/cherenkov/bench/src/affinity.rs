//! CPU placement control for `measure`. Linux and Android only.
//!
//! `--cpu` pins the measuring thread (and, by inheritance, threads
//! spawned afterwards) so timings stay on one cluster of a big.LITTLE
//! host; `--render-cpu` then moves the engine's render thread and the
//! measuring thread to a subset of it. Every thread's mask and CPU time
//! over the measured window is recorded.

use std::collections::BTreeSet;

use crate::BenchError;

/// Parses a `--cpu` list like `7`, `4-6` or `1,3,5-7` into sorted,
/// de-duplicated CPU ids.
///
/// # Errors
/// Returns [`BenchError::Engine`] on malformed input or an empty set.
pub fn parse_cpu_list(spec: &str) -> Result<Vec<u32>, BenchError> {
    let mut set = BTreeSet::new();
    for part in spec.split(',') {
        let part = part.trim();
        let bad = || BenchError::Engine(format!("bad --cpu item {part:?}"));
        if let Some((lo, hi)) = part.split_once('-') {
            let lo: u32 = lo.trim().parse().map_err(|_| bad())?;
            let hi: u32 = hi.trim().parse().map_err(|_| bad())?;
            if hi < lo {
                return Err(BenchError::Engine(format!(
                    "--cpu range {part:?} is reversed"
                )));
            }
            set.extend(lo..=hi);
        } else {
            set.insert(part.parse::<u32>().map_err(|_| bad())?);
        }
    }
    if set.is_empty() {
        return Err(BenchError::Engine("empty --cpu list".into()));
    }
    Ok(set.into_iter().collect())
}

/// Pins the calling thread to `cpus` (`sched_setaffinity`). Threads
/// spawned later inherit the mask, so this must run before the adapter
/// is created to cover its worker threads.
///
/// # Errors
/// Returns [`BenchError::Engine`] when the kernel rejects the set; on
/// platforms without CPU affinity (macOS, Windows) it always errors —
/// the flag is refused rather than silently ignored.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn pin_current_thread(cpus: &[u32]) -> Result<(), BenchError> {
    // The tid 0 selector addresses the calling thread.
    pin_thread(0, cpus)
}

/// Pins the thread `tid` of any process the caller may control
/// (`sched_setaffinity`); 0 is the calling thread.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn pin_thread(tid: libc::pid_t, cpus: &[u32]) -> Result<(), BenchError> {
    // SAFETY: `set` is a valid, fully initialised `cpu_set_t`.
    let rc = unsafe {
        let mut set = std::mem::zeroed::<libc::cpu_set_t>();
        for &cpu in cpus {
            libc::CPU_SET(cpu as usize, &mut set);
        }
        libc::sched_setaffinity(tid, std::mem::size_of_val(&set), &raw const set)
    };
    if rc != 0 {
        return Err(BenchError::Engine(format!(
            "sched_setaffinity({tid}): {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
}

/// Moves the engine's render thread and the calling (measuring) thread
/// to `cpus`; call after the engine is created.
///
/// Every other thread stays where it is: the worker pools the engine
/// built on its render thread keep the mask that was current when the
/// engine was created.
///
/// A thread names itself once it starts, so a worker the render thread
/// has spawned carries the render thread's name until then. The render
/// thread is therefore taken once it is the only thread with its name:
/// every thread it spawned has started.
///
/// # Errors
/// [`BenchError::Engine`] when no thread carries the render thread's
/// name, when more than one still does after [`RENDER_THREAD_START`],
/// or when the kernel rejects the set.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn pin_render_thread(cpus: &[u32]) -> Result<(), BenchError> {
    /// The name the engine gives its render thread
    /// (`cherenkov::Engine::new`, `src/engine/native.rs`).
    const RENDER_THREAD: &str = "cherenkov-render";
    // The kernel keeps a thread's name to TASK_COMM_LEN - 1 = 15 bytes.
    let comm = &RENDER_THREAD[..RENDER_THREAD.len().min(15)];
    let started = std::time::Instant::now();
    loop {
        let named: Vec<ThreadState> = task_threads()?
            .into_iter()
            .filter(|thread| thread.name == comm)
            .collect();
        match named.as_slice() {
            [render] => {
                pin_thread(render.tid, cpus)?;
                return pin_current_thread(cpus);
            }
            [] => {
                return Err(BenchError::Engine(format!(
                    "--render-cpu: no thread is named {comm:?}; the engine has no render thread"
                )));
            }
            _ if started.elapsed() > RENDER_THREAD_START => {
                return Err(BenchError::Engine(format!(
                    "--render-cpu: {} threads are still named {comm:?} after {RENDER_THREAD_START:?}",
                    named.len()
                )));
            }
            _ => std::thread::yield_now(),
        }
    }
}

/// How long the threads the render thread spawned may take to start and
/// name themselves before [`pin_render_thread`] fails.
#[cfg(any(target_os = "linux", target_os = "android"))]
const RENDER_THREAD_START: std::time::Duration = std::time::Duration::from_secs(1);

/// Moves the engine's render thread; unsupported on this platform.
///
/// # Errors
/// Always errors — `--render-cpu` is refused rather than silently ignored.
#[cfg(not(any(target_os = "linux", target_os = "android")))]
pub fn pin_render_thread(_cpus: &[u32]) -> Result<(), BenchError> {
    Err(BenchError::Engine(
        "CPU affinity is not supported on this platform; --render-cpu requires Linux or Android"
            .into(),
    ))
}

/// One thread of this process as `/proc/self/task` shows it.
#[derive(Clone, Debug)]
pub struct ThreadState {
    /// Kernel thread id.
    pub tid: i32,
    /// Thread name (`comm`, at most 15 bytes).
    pub name: String,
    /// CPUs the thread may run on (`Cpus_allowed_list`).
    pub allowed: Vec<u32>,
    /// Nanoseconds the thread has run (`schedstat`'s first field).
    pub run_ns: u64,
}

/// Every thread of this process; `None` where the platform has no
/// per-thread placement (macOS, Windows). Read outside timed frames
/// only: it walks `/proc/self/task`.
///
/// # Errors
/// [`BenchError::Engine`] when a live thread's files cannot be read or
/// parsed. A thread that exits during the walk is skipped.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn threads() -> Result<Option<Vec<ThreadState>>, BenchError> {
    task_threads().map(Some)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn task_threads() -> Result<Vec<ThreadState>, BenchError> {
    let bad = |what: String| BenchError::Engine(format!("/proc/self/task: {what}"));
    let mut out = Vec::new();
    for entry in std::fs::read_dir("/proc/self/task").map_err(|e| bad(e.to_string()))? {
        let path = entry.map_err(|e| bad(e.to_string()))?.path();
        let read = |name: &str| match std::fs::read_to_string(path.join(name)) {
            Ok(text) => Ok(Some(text)),
            // The thread exited between the listing and the read.
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    || error.raw_os_error() == Some(libc::ESRCH) =>
            {
                Ok(None)
            }
            Err(error) => Err(bad(format!("{}/{name}: {error}", path.display()))),
        };
        let (Some(name), Some(status), Some(schedstat)) =
            (read("comm")?, read("status")?, read("schedstat")?)
        else {
            continue;
        };
        let tid = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| bad(format!("{} is not a thread id", path.display())))?;
        let allowed = status
            .lines()
            .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))
            .ok_or_else(|| bad(format!("{tid}: no Cpus_allowed_list")))
            .and_then(|list| parse_cpu_list(list.trim()))?;
        let run_ns = schedstat
            .split_whitespace()
            .next()
            .and_then(|ns| ns.parse().ok())
            .ok_or_else(|| bad(format!("{tid}: schedstat {schedstat:?}")))?;
        out.push(ThreadState {
            tid,
            name: name.trim_end_matches('\n').to_owned(),
            allowed,
            run_ns,
        });
    }
    out.sort_by_key(|thread| thread.tid);
    Ok(out)
}

/// Every thread of this process; `None`: this platform has no
/// per-thread placement.
///
/// # Errors
/// Never.
#[cfg(not(any(target_os = "linux", target_os = "android")))]
pub const fn threads() -> Result<Option<Vec<ThreadState>>, BenchError> {
    Ok(None)
}

/// Pins the calling thread to `cpus`; unsupported on this platform.
///
/// # Errors
/// Always errors — `--cpu` is refused rather than silently ignored.
#[cfg(not(any(target_os = "linux", target_os = "android")))]
pub fn pin_current_thread(_cpus: &[u32]) -> Result<(), BenchError> {
    Err(BenchError::Engine(
        "CPU affinity is not supported on this platform; --cpu requires Linux or Android".into(),
    ))
}

/// The CPU the calling thread is running on (`sched_getcpu`).
#[must_use]
#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn current_cpu() -> Option<u32> {
    // SAFETY: `sched_getcpu` takes no arguments and cannot fail beyond
    // returning -1.
    let cpu = unsafe { libc::sched_getcpu() };
    u32::try_from(cpu).ok()
}

/// The CPU the calling thread is running on; `None` where the kernel
/// does not report it.
#[must_use]
#[cfg(not(any(target_os = "linux", target_os = "android")))]
pub const fn current_cpu() -> Option<u32> {
    None
}

/// `cpuN`'s `cpuinfo_max_freq` in kHz, where sysfs exposes it.
#[must_use]
pub fn cpu_max_freq_khz(cpu: u32) -> Option<u64> {
    std::fs::read_to_string(format!(
        "/sys/devices/system/cpu/cpu{cpu}/cpufreq/cpuinfo_max_freq"
    ))
    .ok()?
    .trim()
    .parse()
    .ok()
}

/// True when the host's CPUs report differing `cpuinfo_max_freq` values —
/// big.LITTLE-class hardware, where unpinned measurements skew across
/// clusters. `false` where sysfs is absent or frequencies agree.
#[must_use]
pub fn cpu_freqs_differ() -> bool {
    let mut freqs = BTreeSet::new();
    if let Ok(dir) = std::fs::read_dir("/sys/devices/system/cpu") {
        for entry in dir.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Some(cpu) = name
                .strip_prefix("cpu")
                .and_then(|rest| rest.parse::<u32>().ok())
            else {
                continue;
            };
            if let Some(freq) = cpu_max_freq_khz(cpu) {
                freqs.insert(freq);
            }
        }
    }
    freqs.len() > 1
}
