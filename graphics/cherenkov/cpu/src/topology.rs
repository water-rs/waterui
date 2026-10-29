//! Resolve the renderer's worker count once, before creating its pool.

use std::num::NonZeroUsize;

use cherenkov::EngineError;

pub fn workers(explicit: Option<usize>) -> Result<NonZeroUsize, EngineError> {
    if let Some(count) = explicit {
        return positive(count, "RasterConfig.threads");
    }
    match std::env::var("RAYON_NUM_THREADS") {
        Ok(value) => {
            let count = value.parse::<usize>().map_err(|error| {
                EngineError::Backend(format!(
                    "RAYON_NUM_THREADS must be a positive integer: {error}"
                ))
            })?;
            positive(count, "RAYON_NUM_THREADS")
        }
        Err(std::env::VarError::NotPresent) => automatic(),
        Err(std::env::VarError::NotUnicode(_)) => Err(EngineError::Backend(
            "RAYON_NUM_THREADS must be a Unicode positive integer".into(),
        )),
    }
}

fn positive(count: usize, source: &str) -> Result<NonZeroUsize, EngineError> {
    NonZeroUsize::new(count)
        .ok_or_else(|| EngineError::Backend(format!("{source} must be positive")))
}

#[cfg(target_vendor = "apple")]
fn automatic() -> Result<NonZeroUsize, EngineError> {
    let mut count: libc::c_int = 0;
    let mut size = std::mem::size_of_val(&count);
    // SAFETY: the name is NUL-terminated, and count/size describe writable
    // storage of the documented integer result. No new value is supplied.
    let result = unsafe {
        libc::sysctlbyname(
            c"hw.perflevel0.physicalcpu".as_ptr(),
            (&raw mut count).cast(),
            &raw mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if result != 0 {
        return Err(EngineError::Backend(format!(
            "hw.perflevel0.physicalcpu: {}",
            std::io::Error::last_os_error()
        )));
    }
    let count = usize::try_from(count)
        .map_err(|error| EngineError::Backend(format!("hw.perflevel0.physicalcpu: {error}")))?;
    positive(count, "hw.perflevel0.physicalcpu")
}

#[cfg(target_os = "android")]
fn automatic() -> Result<NonZeroUsize, EngineError> {
    // SAFETY: cpu_set_t is an integer bitset; all-zero is an empty set.
    let mut allowed: libc::cpu_set_t = unsafe { std::mem::zeroed() };
    // SAFETY: allowed points to writable storage of the stated size.
    let result = unsafe {
        libc::sched_getaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &raw mut allowed)
    };
    if result != 0 {
        return Err(EngineError::Backend(format!(
            "sched_getaffinity: {}",
            std::io::Error::last_os_error()
        )));
    }
    let mut capacities = Vec::new();
    for cpu in 0..libc::CPU_SETSIZE {
        // SAFETY: cpu is within the cpu_set_t bit range.
        if !unsafe { libc::CPU_ISSET(cpu, &allowed) } {
            continue;
        }
        let path = format!("/sys/devices/system/cpu/cpu{cpu}/cpu_capacity");
        let value = std::fs::read_to_string(&path)
            .map_err(|error| EngineError::Backend(format!("{path}: {error}")))?;
        let capacity = value
            .trim()
            .parse::<usize>()
            .map_err(|error| EngineError::Backend(format!("{path}: {error}")))?;
        capacities.push(positive(capacity, &path)?);
    }
    performance_count(&capacities)
}

#[cfg(any(target_os = "android", test))]
fn performance_count(capacities: &[NonZeroUsize]) -> Result<NonZeroUsize, EngineError> {
    let maximum = capacities
        .iter()
        .map(|capacity| capacity.get())
        .max()
        .unwrap_or(0);
    // Include big cores alongside a prime core, not only the fastest class.
    // Half the peak capacity keeps slow cores out of the raster pool while
    // allowing the scheduler to place workers freely within the allowed set.
    let minimum = maximum.div_ceil(2);
    positive(
        capacities
            .iter()
            .filter(|capacity| capacity.get() >= minimum)
            .count(),
        "number of CPUs with at least half the maximum allowed capacity",
    )
}

#[cfg(target_arch = "wasm32")]
fn automatic() -> Result<NonZeroUsize, EngineError> {
    Err(EngineError::Backend(
        "automatic CPU worker discovery is unsupported on WebAssembly".into(),
    ))
}

#[cfg(not(any(target_vendor = "apple", target_os = "android", target_arch = "wasm32")))]
fn automatic() -> Result<NonZeroUsize, EngineError> {
    std::thread::available_parallelism()
        .map_err(|error| EngineError::Backend(format!("available_parallelism: {error}")))
}

#[cfg(test)]
mod tests {
    use super::{performance_count, workers};
    use std::num::NonZeroUsize;

    #[test]
    fn performance_capacity_classes() {
        for (capacities, expected) in [
            (vec![512, 512, 512, 512], 4),
            (vec![188, 188, 188, 188, 731, 731, 731, 1024], 4),
            (vec![1024, 512, 1024, 768], 4),
            (vec![188, 188, 731, 731], 2),
            (vec![511, 512, 1024], 2),
        ] {
            let count = performance_count(
                &capacities
                    .into_iter()
                    .map(|capacity| NonZeroUsize::new(capacity).expect("positive test capacity"))
                    .collect::<Vec<_>>(),
            )
            .expect("nonempty CPU set");
            assert_eq!(count.get(), expected);
        }
        assert!(performance_count(&[]).is_err());
    }

    #[test]
    fn explicit_worker_count_is_positive() {
        assert!(workers(Some(0)).is_err());
        assert_eq!(workers(Some(8)).expect("explicit count").get(), 8);
    }
}
