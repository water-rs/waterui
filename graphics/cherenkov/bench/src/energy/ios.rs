//! The kernel's process energy counter and physical footprint on iOS.
//!
//! `ri_energy_nj` is process-scoped. It must not be described as whole-device
//! energy or added to system counters; the display server is another process.

use std::collections::BTreeMap;
use std::mem::MaybeUninit;
use std::time::Duration;

use crate::BenchError;
use crate::report::{EnergyReport, RailEnergy};

// libc exposes the v4 prefix. The v6 suffix follows sys/resource.h from
// the iPhoneOS SDK; keep the reserved tail so the kernel owns the full buffer.
#[repr(C)]
struct Usage {
    v4: libc::rusage_info_v4,
    flags: u64,
    user_ptime: u64,
    system_ptime: u64,
    pinstructions: u64,
    pcycles: u64,
    energy_nj: u64,
    penergy_nj: u64,
    secure_time_in_system: u64,
    secure_ptime_in_system: u64,
    neural_footprint: u64,
    lifetime_max_neural_footprint: u64,
    interval_max_neural_footprint: u64,
    conclave_footprint: u64,
    page_wait_time_mach: u64,
    page_cache_hits: u64,
    reserved: [u64; 6],
}

/// One kernel snapshot of the calling process.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Snapshot {
    /// Cumulative process energy in nanojoules.
    pub energy_nj: u64,
    /// Current physical footprint in bytes.
    pub phys_footprint: u64,
    /// Lifetime maximum physical footprint in bytes.
    pub peak_phys_footprint: u64,
}

/// Reads the process's actual counters, failing if the kernel rejects v6.
///
/// # Errors
/// The kernel error returned by `proc_pid_rusage`.
pub fn snapshot() -> Result<Snapshot, BenchError> {
    let mut usage = MaybeUninit::<Usage>::uninit();
    // SAFETY: v6 receives the complete SDK-defined layout, and the kernel
    // initializes that buffer on success. getpid names this process.
    let result = unsafe { libc::proc_pid_rusage(libc::getpid(), 6, usage.as_mut_ptr().cast()) };
    if result != 0 {
        return Err(BenchError::Engine(format!(
            "energy: proc_pid_rusage v6: {}",
            std::io::Error::last_os_error()
        )));
    }
    // SAFETY: successful proc_pid_rusage initialized every requested field.
    let usage = unsafe { usage.assume_init() };
    Ok(Snapshot {
        energy_nj: usage.energy_nj,
        phys_footprint: usage.v4.ri_phys_footprint,
        peak_phys_footprint: usage.v4.ri_lifetime_max_phys_footprint,
    })
}

#[expect(
    clippy::cast_precision_loss,
    reason = "energy is reported in floating-point joules"
)]
pub(super) fn report(
    before: &Snapshot,
    after: &Snapshot,
    window: Duration,
    frames: u32,
) -> Result<EnergyReport, BenchError> {
    let delta = after
        .energy_nj
        .checked_sub(before.energy_nj)
        .ok_or_else(|| {
            BenchError::Engine("energy: the iOS process energy counter went backwards".into())
        })?;
    if delta == 0 {
        return Err(BenchError::Engine(
            "energy: iOS returned no process energy over the measured window".into(),
        ));
    }
    let joules = delta as f64 / 1e9;
    let seconds = window.as_secs_f64();
    let per_frame = joules / f64::from(frames);
    let watts = joules / seconds;
    Ok(EnergyReport {
        source: "proc_pid_rusage_v6",
        window_seconds: seconds,
        rails: BTreeMap::from([(
            "process".into(),
            RailEnergy {
                subsystem: Some("process".into()),
                joules,
                joules_per_frame: per_frame,
                watts,
            },
        )]),
        total_joules: joules,
        joules_per_frame: per_frame,
        watts,
    })
}
