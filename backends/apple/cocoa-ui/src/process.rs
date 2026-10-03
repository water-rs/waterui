//! The running process: its name and how long ago the kernel started it.
//!
//! # Safety
//!
//! The `unsafe` here calls into the kernel with buffers this module owns and
//! sizes itself: `proc_pid_rusage` and `mach_timebase_info` on macOS, `sysctl`
//! on iOS. Each call is made for the current process only.

use std::io;
use std::time::Duration;

use objc2_foundation::NSProcessInfo;

/// The process's name, as the system reports it.
#[must_use]
pub fn name() -> String {
    NSProcessInfo::processInfo().processName().to_string()
}

/// How long ago the kernel started this process.
///
/// The start is the kernel's own record, so the interval covers everything
/// that ran before any Rust code did: `dyld`, static initializers, and the
/// frameworks' own launch work. On macOS both ends are read from the
/// monotonic clock; iOS keeps the start only as a wall-clock time, so there
/// both ends are wall-clock times.
///
/// # Errors
///
/// If the kernel refuses to report this process's start, or, on iOS, if the
/// wall clock has been set back to before the process started.
///
/// # Panics
///
/// If the kernel reports a start time that cannot be one: later than the
/// current reading of the monotonic clock, or before 1970.
pub fn time_since_start() -> io::Result<Duration> {
    platform::time_since_start()
}

#[cfg(target_os = "macos")]
mod platform {
    use std::io;
    use std::mem::MaybeUninit;
    use std::time::Duration;

    use mach2::mach_time::{mach_absolute_time, mach_timebase_info};

    pub fn time_since_start() -> io::Result<Duration> {
        let mut usage = MaybeUninit::<libc::rusage_info_v4>::uninit();
        // SAFETY: `usage` is a `rusage_info_v4` the kernel fills for the
        // `RUSAGE_INFO_V4` flavor; see the module safety note.
        let status = unsafe {
            libc::proc_pid_rusage(
                libc::getpid(),
                libc::RUSAGE_INFO_V4,
                usage.as_mut_ptr().cast::<libc::rusage_info_t>(),
            )
        };
        if status != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the call above succeeded, so the kernel initialized `usage`.
        let started = unsafe { usage.assume_init() }.ri_proc_start_abstime;

        let mut timebase = mach2::mach_time::mach_timebase_info_data_t::default();
        // SAFETY: `timebase` is the struct the call fills; see the module
        // safety note.
        let status = unsafe { mach_timebase_info(&raw mut timebase) };
        if status != mach2::kern_return::KERN_SUCCESS {
            return Err(io::Error::other(format!(
                "mach_timebase_info failed with kern_return_t {status}"
            )));
        }
        // SAFETY: reading the absolute clock has no preconditions.
        let now = unsafe { mach_absolute_time() };
        let elapsed = now
            .checked_sub(started)
            .expect("the monotonic clock must not read earlier than the process start");
        Ok(Duration::from_nanos(nanos_from_ticks(
            elapsed,
            timebase.numer,
            timebase.denom,
        )))
    }

    /// Nanoseconds from `ticks` of the absolute clock, whose tick lasts
    /// `numer / denom` nanoseconds.
    ///
    /// The product is taken in 128 bits: on Apple silicon `ticks * numer`
    /// leaves 64 bits long before the quotient does.
    pub fn nanos_from_ticks(ticks: u64, numer: u32, denom: u32) -> u64 {
        let nanos = u128::from(ticks) * u128::from(numer) / u128::from(denom);
        u64::try_from(nanos).expect("the absolute clock reading must fit in 64 bits of nanoseconds")
    }
}

#[cfg(target_os = "ios")]
mod platform {
    use std::io;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    pub fn time_since_start() -> io::Result<Duration> {
        let started = UNIX_EPOCH + process_start_since_epoch()?;
        SystemTime::now()
            .duration_since(started)
            .map_err(|_| io::Error::other("the wall clock reads earlier than the process start"))
    }

    /// The process start, as `sysctl(KERN_PROC_PID)` reports it.
    ///
    /// `libproc`, which reports the start on the monotonic clock, is not
    /// public on iOS. The kernel's process record is: its first field is the
    /// start time as a `timeval`, so the record is read into a buffer the
    /// kernel sizes and only that field is taken from it.
    fn process_start_since_epoch() -> io::Result<Duration> {
        // SAFETY: `getpid` has no preconditions.
        let pid = unsafe { libc::getpid() };
        let mut mib = [libc::CTL_KERN, libc::KERN_PROC, libc::KERN_PROC_PID, pid];
        let mut size = 0;
        // SAFETY: a null buffer asks the kernel only for the record's size;
        // see the module safety note.
        let status = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                4,
                std::ptr::null_mut(),
                &raw mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if status != 0 {
            return Err(io::Error::last_os_error());
        }
        assert!(
            size >= size_of::<libc::timeval>(),
            "the kernel's process record ({size} bytes) is smaller than its start time"
        );
        // `u64` words keep the buffer aligned for the `timeval` read below.
        let mut record = vec![0_u64; size.div_ceil(size_of::<u64>())];
        // SAFETY: `record` holds at least `size` bytes; see the module safety
        // note.
        let status = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                4,
                record.as_mut_ptr().cast(),
                &raw mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if status != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the record starts with `kp_proc.p_starttime`, a `timeval`,
        // and `record` is aligned for it and at least that long.
        let started = unsafe { record.as_ptr().cast::<libc::timeval>().read() };
        let seconds = u64::try_from(started.tv_sec)
            .expect("the kernel must report a process start after the epoch");
        let micros = u32::try_from(started.tv_usec)
            .expect("the kernel must report a sub-second part within one second");
        Ok(Duration::from_secs(seconds) + Duration::from_micros(u64::from(micros)))
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use std::time::Duration;

    use super::platform::nanos_from_ticks;
    use super::time_since_start;

    #[test]
    fn a_test_process_started_moments_ago() {
        let elapsed = time_since_start().expect("the kernel reports its own process");
        assert!(
            elapsed > Duration::ZERO && elapsed < Duration::from_secs(600),
            "the test process reports having started {elapsed:?} ago"
        );
    }

    #[test]
    fn converts_ticks_with_the_timebase() {
        // Apple silicon: 125/3 ns per tick.
        assert_eq!(nanos_from_ticks(24_000_000, 125, 3), 1_000_000_000);
        // Intel: one tick per nanosecond.
        assert_eq!(nanos_from_ticks(42, 1, 1), 42);
    }

    #[test]
    fn keeps_the_quotient_when_the_product_leaves_64_bits() {
        let ticks = u64::MAX / 64;
        let expected = u128::from(ticks) * 125 / 3;
        assert_eq!(
            u128::from(nanos_from_ticks(ticks, 125, 3)),
            expected,
            "{ticks} ticks"
        );
    }
}
