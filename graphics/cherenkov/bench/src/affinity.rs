//! CPU placement control for `measure`: `--cpu` pins the measuring
//! thread (and, by inheritance, threads spawned afterwards) so timings
//! stay on one cluster of a big.LITTLE host. Linux and Android only.

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
    // SAFETY: `set` is a valid, fully initialised `cpu_set_t`; the pid 0
    // selector addresses the calling thread.
    let rc = unsafe {
        let mut set = std::mem::zeroed::<libc::cpu_set_t>();
        for &cpu in cpus {
            libc::CPU_SET(cpu as usize, &mut set);
        }
        libc::sched_setaffinity(0, std::mem::size_of_val(&set), &raw const set)
    };
    if rc != 0 {
        return Err(BenchError::Engine(format!(
            "sched_setaffinity: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
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
