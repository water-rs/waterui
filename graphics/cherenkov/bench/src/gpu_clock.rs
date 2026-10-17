//! GPU clock residency over a `measure` window.
//!
//! A GPU's busy time for a fixed workload scales with its clock, and a
//! DVFS governor picks the clock from recent utilization, so a GPU time
//! means little without the clocks it ran at (#2613). Where the kernel
//! keeps a per-clock residency table, the window's residency is the
//! difference of two reads of it, one as the window opens and one as it
//! closes.
//!
//! - **Android, Mali**: the Pixel GPU driver's `time_in_state` on the
//!   Mali device (`/sys/class/misc/mali0/device`): one line per shader
//!   clock, `kHz milliseconds`, counting time the GPU was powered at
//!   that clock. Root-only, like the energy rails.
//!
//! A platform without such a table reports no residency; a table that is
//! present but does not parse is an error, never a silently absent field.

use crate::BenchError;
use crate::report::{ClockResidency, GpuClockReport};

/// The residency table the window is measured against.
#[cfg(target_os = "android")]
const TIME_IN_STATE: &str = "/sys/class/misc/mali0/device/time_in_state";

/// One read of the residency table: milliseconds per clock in kHz.
#[derive(Debug)]
pub struct Snapshot(Vec<(u64, u64)>);

impl Snapshot {
    /// Reads the table, or `None` where the platform keeps none.
    ///
    /// # Errors
    /// A table that exists but cannot be read or parsed.
    #[cfg(target_os = "android")]
    pub fn read() -> Result<Option<Self>, BenchError> {
        match std::fs::read_to_string(TIME_IN_STATE) {
            Ok(text) => parse(&text).map(|table| Some(Self(table))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(BenchError::Engine(format!(
                "gpu clock: {TIME_IN_STATE}: {e}"
            ))),
        }
    }

    /// No residency table on this platform.
    ///
    /// # Errors
    /// Never.
    #[cfg(not(target_os = "android"))]
    pub const fn read() -> Result<Option<Self>, BenchError> {
        Ok(None)
    }

    /// The residency between `self` and the later read `end`.
    ///
    /// # Errors
    /// The two reads list different clocks or a counter went backwards.
    pub fn until(&self, end: &Self) -> Result<GpuClockReport, BenchError> {
        if self.0.len() != end.0.len() {
            return Err(BenchError::Engine(
                "gpu clock: the residency table changed shape during the window".into(),
            ));
        }
        let residency = self
            .0
            .iter()
            .zip(&end.0)
            .map(|(&(khz, before), &(end_khz, after))| {
                if khz != end_khz || after < before {
                    return Err(BenchError::Engine(format!(
                        "gpu clock: {khz} kHz read {before} ms then {end_khz} kHz {after} ms"
                    )));
                }
                Ok(ClockResidency {
                    khz,
                    milliseconds: after - before,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(GpuClockReport { residency })
    }
}

/// Parses `kHz milliseconds` lines.
#[cfg(any(target_os = "android", test))]
fn parse(text: &str) -> Result<Vec<(u64, u64)>, BenchError> {
    let table = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let mut fields = line.split_whitespace().map(str::parse::<u64>);
            match (fields.next(), fields.next(), fields.next()) {
                (Some(Ok(khz)), Some(Ok(ms)), None) => Ok((khz, ms)),
                _ => Err(BenchError::Engine(format!(
                    "gpu clock: unexpected residency line {line:?}"
                ))),
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    if table.is_empty() {
        return Err(BenchError::Engine(
            "gpu clock: empty residency table".into(),
        ));
    }
    Ok(table)
}

#[cfg(test)]
mod tests {
    use super::{Snapshot, parse};

    #[test]
    fn residency_is_the_difference_of_two_reads() {
        let before = Snapshot(parse("  940000      10\n  649000     200\n").unwrap());
        let after = Snapshot(parse("  940000      10\n  649000    1201\n").unwrap());
        let report = before.until(&after).unwrap();
        let rows: Vec<_> = report
            .residency
            .iter()
            .map(|r| (r.khz, r.milliseconds))
            .collect();
        assert_eq!(rows, [(940_000, 0), (649_000, 1001)]);
    }

    #[test]
    fn malformed_tables_fail() {
        assert!(parse("").is_err());
        assert!(parse("649000\n").is_err());
        assert!(parse("649000 1 2\n").is_err());
        let a = Snapshot(vec![(649_000, 5)]);
        assert!(a.until(&Snapshot(vec![(649_000, 4)])).is_err());
        assert!(a.until(&Snapshot(vec![(521_000, 9)])).is_err());
    }
}
