//! Energy metering for `measure --energy`.
//!
//! Platform sources feed the same [`EnergyReport`]:
//!
//! - **Android ODPM** — the on-device power monitors under
//!   `/sys/bus/iio/devices/iio:device*/` (Pixel-class devices). Every
//!   rail named in `enabled_rails` is sampled once before the first
//!   measured frame and once after the last; the per-rail deltas are
//!   the window's energy. The rails are root-only, so the bench runs
//!   via `su -c`.
//! - **macOS `powermetrics`** — `sudo -n powermetrics --samplers
//!   cpu_power,gpu_power -i <ms> --format plist` spawned for exactly the
//!   measured window; its per-sample CPU/GPU/ANE energies are summed.
//! - **iOS process counter** — `proc_pid_rusage` v6 reports cumulative
//!   `ri_energy_nj`. This measures the calling process, excluding the
//!   display and other processes such as the system compositor.
//!
//! [`Meter::probe`] runs before the engine is created so `--energy`
//! fails fast — naming the path and the permission — when the meter
//! cannot be read, rather than producing a report without energy.

use std::time::{Duration, Instant};

use crate::BenchError;
use crate::report::{EnergyReport, RailEnergy};

#[cfg(target_os = "ios")]
pub mod ios;

/// The meter open for one measured window.
#[derive(Debug)]
pub enum Meter {
    /// Android ODPM: holds the pre-window rail snapshot and the instant
    /// the window opened.
    Odpm(odpm::Snapshot, Instant),
    /// macOS `powermetrics`: the running subprocess.
    PowerMetrics(powermetrics::Run),
    /// iOS process energy counter, in nanojoules, sampled at window boundaries.
    #[cfg(target_os = "ios")]
    Ios(ios::Snapshot),
}

/// The result of a finished metering window.
#[derive(Debug)]
pub struct Outcome {
    /// The serialized energy report.
    pub report: EnergyReport,
    /// `powermetrics` `thermal_pressure` from its last decoded sample —
    /// the only thermal-status source on macOS. `None` on Android and iOS
    /// (conditions are read separately).
    pub thermal_pressure: Option<String>,
}

#[cfg(not(target_os = "ios"))]
fn unsupported() -> BenchError {
    BenchError::Engine(
        "energy: --energy is not supported on this platform \
         (Android ODPM rails, macOS powermetrics or iOS process counters required)"
            .into(),
    )
}

impl Meter {
    /// Validates that this platform's meter can be read, before the
    /// engine or the scene is touched — `--energy` fails here, not
    /// mid-run.
    ///
    /// # Errors
    /// [`BenchError::Engine`] naming the path and the permission that
    /// blocks the meter: the `iio:device*` rail files on Android,
    /// `sudo -n` on macOS, `proc_pid_rusage` on iOS, or the platform itself elsewhere.
    pub fn probe() -> Result<(), BenchError> {
        #[cfg(target_os = "ios")]
        {
            ios::snapshot().map(|_| ())
        }
        #[cfg(not(target_os = "ios"))]
        if cfg!(target_os = "android") {
            odpm::snapshot().map(|_| ())
        } else if cfg!(target_os = "macos") {
            powermetrics::check_sudo()
        } else {
            Err(unsupported())
        }
    }

    /// Opens the window: snapshots the ODPM rails or spawns
    /// `powermetrics`.
    ///
    /// `window_hint` estimates the window's length; `powermetrics` uses
    /// it to pick a sampling interval.
    ///
    /// # Errors
    /// Same as [`Meter::probe`].
    pub fn begin(window_hint: Duration) -> Result<Self, BenchError> {
        #[cfg(target_os = "ios")]
        {
            let _ = window_hint;
            ios::snapshot().map(Self::Ios)
        }
        #[cfg(not(target_os = "ios"))]
        if cfg!(target_os = "android") {
            Ok(Self::Odpm(odpm::snapshot()?, Instant::now()))
        } else if cfg!(target_os = "macos") {
            powermetrics::Run::spawn(window_hint).map(Self::PowerMetrics)
        } else {
            Err(unsupported())
        }
    }

    /// Closes the window and integrates the energy into a report.
    ///
    /// `start`/`end` bound the measured window; the powermetrics path
    /// attributes sample energy to it by interval overlap (the ODPM
    /// path diffs counters read at the window's edges instead).
    ///
    /// # Errors
    /// [`BenchError::Engine`] when `frames` is zero or the post-window
    /// read fails.
    pub fn finish(self, start: Instant, end: Instant, frames: u32) -> Result<Outcome, BenchError> {
        if frames == 0 {
            return Err(BenchError::Engine(
                "energy: --energy requires at least one measured frame (--frames > 0)".into(),
            ));
        }
        match self {
            Self::Odpm(before, opened) => {
                let after = odpm::snapshot()?;
                let window = before
                    .timestamp_ms
                    .zip(after.timestamp_ms)
                    .and_then(|(a, b)| b.checked_sub(a))
                    .map_or_else(|| opened.elapsed(), Duration::from_millis);
                Ok(Outcome {
                    report: odpm::diff_report(&before, &after, window, frames)?,
                    thermal_pressure: None,
                })
            }
            Self::PowerMetrics(run) => run.finish(start, end, frames),
            #[cfg(target_os = "ios")]
            Self::Ios(before) => Ok(Outcome {
                report: ios::report(&before, &ios::snapshot()?, end - start, frames)?,
                thermal_pressure: None,
            }),
        }
    }
}

/// Android on-device power monitor (ODPM) sysfs access.
///
/// The Pixel kernel exposes power-monitor IIO devices under
/// `/sys/bus/iio/devices/iio:device*/`, each with:
///
/// - `enabled_rails` — one `CH<idx>[<RAIL>]:<subsystem>` line per
///   enabled rail, e.g. `CH2[VSYS_PWR_RFFE]:Cellular` (the format AOSP's
///   `IioEnergyMeterDataProvider::parseEnabledRails` decodes).
/// - `energy_value` — a `t=<ms>` boot-time timestamp line followed by
///   one `CH<idx>(T=<ms>)[<RAIL>], <µWs>` line per channel, the rail's
///   cumulative energy in microwatt-seconds (`parseEnergyContents`).
pub mod odpm {
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::time::Duration;

    use super::{BenchError, EnergyReport, RailEnergy};

    /// Root of the ODPM device nodes.
    pub const ROOT: &str = "/sys/bus/iio/devices";

    /// A snapshot of every rail's cumulative energy.
    #[derive(Debug)]
    pub struct Snapshot {
        /// Cumulative µWs per rail name.
        pub rails: BTreeMap<String, u64>,
        /// Subsystem label per rail name, from `enabled_rails`.
        pub subsystems: BTreeMap<String, String>,
        /// Latest `t=` timestamp seen across the devices, ms from boot.
        pub timestamp_ms: Option<u64>,
    }

    /// Parses one `enabled_rails` file into `(channel, rail, subsystem)`
    /// rows.
    ///
    /// Lines look like `CH2[VSYS_PWR_RFFE]:Cellular`; any line that
    /// doesn't fit is skipped (matching AOSP's tolerance of malformed
    /// rows).
    #[must_use]
    pub fn parse_enabled_rails(contents: &str) -> Vec<(u32, String, String)> {
        let mut rows = Vec::new();
        for line in contents.lines() {
            // `CH2[VSYS_PWR_RFFE]:Cellular` splits on `:`, `]`, `[` into
            // `["CH2", "VSYS_PWR_RFFE", "", "Cellular"]`.
            let words: Vec<&str> = line.split([':', ']', '[']).collect();
            if words.len() == 4
                && let Some(channel) = words[0].strip_prefix("CH").and_then(|c| c.parse().ok())
            {
                rows.push((channel, words[1].to_owned(), words[3].to_owned()));
            }
        }
        rows
    }

    /// One parsed `energy_value` file.
    #[derive(Debug)]
    pub struct EnergyValue {
        /// The `t=` timestamp, ms from boot.
        pub timestamp_ms: u64,
        /// `(channel, rail, cumulative µWs)` per channel line.
        pub rows: Vec<(u32, String, u64)>,
    }

    /// Parses one `energy_value` file.
    ///
    /// # Errors
    /// [`BenchError::Engine`] when any line doesn't match the documented
    /// format — a malformed file is a read failure, not a zero.
    pub fn parse_energy_value(contents: &str) -> Result<EnergyValue, BenchError> {
        let malformed = |line: &str| {
            BenchError::Engine(format!("energy: malformed energy_value line {line:?}"))
        };
        let mut timestamp = None;
        let mut rows = Vec::new();
        for line in contents.lines() {
            if timestamp.is_none() {
                // First line is `t=<ms since boot>`.
                let t = line
                    .strip_prefix("t=")
                    .and_then(|v| v.trim().parse::<u64>().ok())
                    .ok_or_else(|| malformed(line))?;
                timestamp = Some(t);
                continue;
            }
            // `CH3(T=358356)[S2M_VDD_CPUCL2], 761330`
            let row = (|| {
                let rest = line.strip_prefix("CH")?;
                let (channel, rest) = rest.split_once('(')?;
                let rest = rest.strip_prefix("T=")?;
                let (_duration, rest) = rest.split_once(')')?;
                let rest = rest.strip_prefix('[')?;
                let (rail, rest) = rest.split_once(']')?;
                Some((
                    channel.parse::<u32>().ok()?,
                    rail.to_owned(),
                    rest.trim_start_matches([',', ' ']).parse::<u64>().ok()?,
                ))
            })();
            rows.push(row.ok_or_else(|| malformed(line))?);
        }
        let timestamp =
            timestamp.ok_or_else(|| BenchError::Engine("energy: energy_value is empty".into()))?;
        Ok(EnergyValue {
            timestamp_ms: timestamp,
            rows,
        })
    }

    /// Reads every ODPM device under [`ROOT`].
    ///
    /// # Errors
    /// See [`snapshot_at`].
    pub fn snapshot() -> Result<Snapshot, BenchError> {
        snapshot_at(Path::new(ROOT))
    }

    /// Reads every `iio:device*` directory under `root` that carries
    /// both `enabled_rails` and `energy_value`.
    ///
    /// # Errors
    /// [`BenchError::Engine`] naming the path and the OS error when the
    /// root can't be listed, when no ODPM device is present, or when a
    /// rail file can't be read — the rails are root-only.
    pub fn snapshot_at(root: &Path) -> Result<Snapshot, BenchError> {
        let io_err = |path: &Path, verb: &str, e: &std::io::Error| {
            BenchError::Engine(format!(
                "energy: cannot {verb} {}: {e} — the ODPM rails are root-only; \
                 run via `su -c`",
                path.display()
            ))
        };
        let rd = std::fs::read_dir(root).map_err(|e| io_err(root, "list", &e))?;
        let mut devices = Vec::new();
        for entry in rd {
            let entry = entry.map_err(|e| io_err(root, "list", &e))?;
            let name = entry.file_name();
            if !name.to_string_lossy().starts_with("iio:device") {
                continue;
            }
            let dir = entry.path();
            if dir.join("enabled_rails").is_file() && dir.join("energy_value").is_file() {
                devices.push(dir);
            }
        }
        devices.sort();
        if devices.is_empty() {
            return Err(BenchError::Engine(format!(
                "energy: no ODPM power monitors under {} (expected \
                 iio:device*/enabled_rails and energy_value); a \
                 Pixel-class device and root are required",
                root.display()
            )));
        }
        let mut rails = BTreeMap::new();
        let mut subsystems = BTreeMap::new();
        let mut timestamp_ms = None;
        for dev in devices {
            let dev_name = dev
                .file_name()
                .map_or_else(|| "device".into(), |n| n.to_string_lossy().into_owned());
            let rails_path = dev.join("enabled_rails");
            let contents = std::fs::read_to_string(&rails_path)
                .map_err(|e| io_err(&rails_path, "read", &e))?;
            for (_channel, rail, subsystem) in parse_enabled_rails(&contents) {
                subsystems.insert(rail, subsystem);
            }
            let energy_path = dev.join("energy_value");
            let contents = std::fs::read_to_string(&energy_path)
                .map_err(|e| io_err(&energy_path, "read", &e))?;
            let value = parse_energy_value(&contents).map_err(|e| {
                BenchError::Engine(format!("energy: {}: {e}", energy_path.display()))
            })?;
            timestamp_ms = timestamp_ms.max(Some(value.timestamp_ms));
            for (_channel, rail, uws) in value.rows {
                // Rail names are unique per device; if a name repeats
                // across devices, qualify the second occurrence.
                let key = if rails.contains_key(&rail) {
                    format!("{rail}@{dev_name}")
                } else {
                    rail.clone()
                };
                if key != rail
                    && let Some(subsystem) = subsystems.get(&rail).cloned()
                {
                    subsystems.insert(key.clone(), subsystem);
                }
                rails.insert(key, uws);
            }
        }
        if rails.is_empty() {
            return Err(BenchError::Engine(
                "energy: the ODPM devices list no rails in energy_value".into(),
            ));
        }
        Ok(Snapshot {
            rails,
            subsystems,
            timestamp_ms,
        })
    }

    /// Builds the energy report from the before/after snapshots over
    /// `window` seconds of `frames` measured frames.
    ///
    /// # Errors
    /// [`BenchError::Engine`] naming the rails that appeared or
    /// disappeared when the two snapshots' rail sets differ — a
    /// mismatched diff cannot be reported.
    #[expect(
        clippy::cast_precision_loss,
        reason = "µWs deltas fit f64 for any real window"
    )]
    pub fn diff_report(
        before: &Snapshot,
        after: &Snapshot,
        window: Duration,
        frames: u32,
    ) -> Result<EnergyReport, BenchError> {
        let appeared: Vec<&str> = after
            .rails
            .keys()
            .filter(|rail| !before.rails.contains_key(*rail))
            .map(String::as_str)
            .collect();
        let disappeared: Vec<&str> = before
            .rails
            .keys()
            .filter(|rail| !after.rails.contains_key(*rail))
            .map(String::as_str)
            .collect();
        if !appeared.is_empty() || !disappeared.is_empty() {
            use std::fmt::Write as _;
            let mut detail = String::new();
            if !appeared.is_empty() {
                let _ = write!(detail, " (appeared: {})", appeared.join(", "));
            }
            if !disappeared.is_empty() {
                let _ = write!(detail, " (disappeared: {})", disappeared.join(", "));
            }
            return Err(BenchError::Engine(format!(
                "energy: the ODPM rail set changed during the window{detail}"
            )));
        }
        let window_s = window.as_secs_f64();
        let frames = f64::from(frames);
        let mut rails = BTreeMap::new();
        let mut total_joules = 0.0f64;
        for (rail, &uws) in &after.rails {
            let joules = uws.saturating_sub(before.rails[rail]) as f64 / 1e6;
            total_joules += joules;
            rails.insert(
                rail.clone(),
                RailEnergy {
                    subsystem: after.subsystems.get(rail).cloned(),
                    joules,
                    joules_per_frame: joules / frames,
                    watts: joules / window_s,
                },
            );
        }
        Ok(EnergyReport {
            source: "odpm",
            window_seconds: window_s,
            rails,
            total_joules,
            joules_per_frame: total_joules / frames,
            watts: total_joules / window_s,
        })
    }
}

/// macOS `powermetrics` energy metering (root required).
///
/// `sudo -n powermetrics --samplers cpu_power,gpu_power -i <ms>
/// --format plist` emits one NUL-separated XML plist per sample. The
/// `processor` dictionary carries per-sample energies in mJ —
/// `cpu_energy`, `gpu_energy`, `ane_energy`, `dram_energy` on Apple
/// silicon, `package_joules` on Intel — plus the package total
/// (`package_energy`/`package_joules`) and a `thermal_pressure` word.
///
/// Samples are placed on a timeline anchored at the spawn, each
/// covering `elapsed_ns`; energy inside the measured window is found
/// by overlap — boundary samples are scaled to the fraction of their
/// interval inside it. The process is stopped with SIGTERM (which
/// `sudo` forwards so `powermetrics` flushes and exits) only after a
/// sample boundary past the window's end has arrived.
pub mod powermetrics {
    use std::collections::{BTreeMap, VecDeque};
    use std::io::Read;
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
    use std::time::{Duration, Instant};

    use super::{BenchError, EnergyReport, Outcome, RailEnergy};

    /// One decoded `powermetrics` interval on the spawn timeline.
    #[derive(Debug)]
    pub(crate) struct Sample {
        /// Interval bounds as offsets from the meter's spawn.
        pub start: Duration,
        /// Interval end as an offset from the meter's spawn.
        pub end: Duration,
        /// Joules per domain over the interval (`cpu`, `gpu`, `ane`,
        /// `dram`).
        pub joules: BTreeMap<String, f64>,
        /// Package joules over the interval when reported — the sum
        /// of the domains, used as the report's total rather than a
        /// rail of its own.
        pub package: Option<f64>,
        /// `thermal_pressure` decoded from this sample.
        pub thermal_pressure: Option<String>,
    }

    /// The window's energies after boundary clipping.
    #[derive(Debug, Default)]
    pub(crate) struct Attributed {
        /// Joules per domain attributed to the window.
        pub joules: BTreeMap<String, f64>,
        /// Package joules attributed to the window, when the platform
        /// reports a package counter.
        pub package: Option<f64>,
        /// `thermal_pressure` from the last sample overlapping the
        /// window.
        pub thermal_pressure: Option<String>,
    }

    #[derive(serde::Deserialize)]
    struct Doc {
        #[serde(default)]
        elapsed_ns: u64,
        #[serde(default)]
        processor: Option<Processor>,
        #[serde(default)]
        thermal_pressure: Option<String>,
    }

    #[derive(serde::Deserialize)]
    struct Processor {
        /// mJ over the sample period (Apple silicon).
        #[serde(default)]
        cpu_energy: Option<f64>,
        #[serde(default)]
        gpu_energy: Option<f64>,
        #[serde(default)]
        ane_energy: Option<f64>,
        #[serde(default)]
        dram_energy: Option<f64>,
        #[serde(default)]
        package_energy: Option<f64>,
        /// Joules over the sample period (Intel).
        #[serde(default)]
        package_joules: Option<f64>,
    }

    /// Decodes one plist document into a [`Sample`] whose interval is
    /// `[offset, offset + elapsed_ns]` on the spawn timeline. Documents
    /// that fail to decode — e.g. the truncated tail after a kill —
    /// return `None`.
    pub(crate) fn parse_doc(chunk: &[u8], offset: Duration) -> Option<Sample> {
        let doc = plist::from_bytes::<Doc>(chunk).ok()?;
        let end = offset + Duration::from_nanos(doc.elapsed_ns);
        let mut joules = BTreeMap::new();
        let package = doc.processor.and_then(|p| {
            // Apple silicon reports mJ per domain plus the package
            // total; Intel reports only the package, in joules, which
            // maps onto `cpu`.
            let mut mj = |name: &str, value: Option<f64>| {
                if let Some(v) = value {
                    *joules.entry(name.to_owned()).or_default() += v / 1e3;
                }
            };
            mj(
                "cpu",
                p.cpu_energy.or_else(|| p.package_joules.map(|j| j * 1e3)),
            );
            mj("gpu", p.gpu_energy);
            mj("ane", p.ane_energy);
            mj("dram", p.dram_energy);
            p.package_energy.map(|v| v / 1e3).or(p.package_joules)
        });
        Some(Sample {
            start: offset,
            end,
            joules,
            package,
            thermal_pressure: doc.thermal_pressure,
        })
    }

    /// Decodes a whole captured stream — used by tests; the live path
    /// consumes documents incrementally in [`Run::collect_samples`].
    #[cfg(test)]
    pub(crate) fn parse_stream(stream: &[u8]) -> Vec<Sample> {
        let mut offset = Duration::ZERO;
        let mut out = Vec::new();
        for chunk in stream.split(|&b| b == 0) {
            if chunk.is_empty() {
                continue;
            }
            if let Some(sample) = parse_doc(chunk, offset) {
                offset = sample.end;
                out.push(sample);
            }
        }
        out
    }

    /// Scales each sample by the fraction of its interval inside
    /// `[window_start, window_end]` (offsets from the spawn) and sums
    /// per domain and for the package counter separately. Samples
    /// entirely outside the window contribute nothing.
    #[must_use]
    pub(crate) fn attribute(
        samples: &[Sample],
        window_start: Duration,
        window_end: Duration,
    ) -> Attributed {
        let mut out = Attributed::default();
        for sample in samples {
            let inside = sample
                .end
                .min(window_end)
                .saturating_sub(sample.start.max(window_start));
            let interval = sample.end.saturating_sub(sample.start);
            if inside.is_zero() || interval.is_zero() {
                continue;
            }
            let fraction = inside.as_secs_f64() / interval.as_secs_f64();
            for (name, &joules) in &sample.joules {
                *out.joules.entry(name.clone()).or_default() =
                    joules.mul_add(fraction, *out.joules.entry(name.clone()).or_default());
            }
            if let Some(package) = sample.package {
                *out.package.get_or_insert(0.0) =
                    package.mul_add(fraction, *out.package.get_or_insert(0.0));
            }
            if sample.thermal_pressure.is_some() {
                out.thermal_pressure.clone_from(&sample.thermal_pressure);
            }
        }
        out
    }

    /// Builds the report from the attributed window energy.
    ///
    /// The domains are the rails; the package counter, when present, is
    /// the total — on Apple silicon it already sums the domains, so
    /// reporting it as a rail too would count the window twice.
    fn report(attributed: &Attributed, window_s: f64, frames: u32) -> EnergyReport {
        let frames = f64::from(frames);
        let mut domains = 0.0;
        let rails = attributed
            .joules
            .iter()
            .map(|(name, &joules)| {
                domains += joules;
                (
                    name.clone(),
                    RailEnergy {
                        subsystem: None,
                        joules,
                        joules_per_frame: joules / frames,
                        watts: joules / window_s,
                    },
                )
            })
            .collect();
        let total_joules = attributed.package.unwrap_or(domains);
        EnergyReport {
            source: "powermetrics",
            window_seconds: window_s,
            rails,
            total_joules,
            joules_per_frame: total_joules / frames,
            watts: total_joules / window_s,
        }
    }

    /// Checks that `sudo -n` is permitted — the `powermetrics`
    /// precondition — before the measured window opens.
    ///
    /// # Errors
    /// [`BenchError::Engine`] when `sudo` can't run or refuses
    /// non-interactive use.
    pub fn check_sudo() -> Result<(), BenchError> {
        let out = Command::new("sudo")
            .args(["-n", "true"])
            .output()
            .map_err(|e| BenchError::Engine(format!("energy: cannot run `sudo -n`: {e}")))?;
        if out.status.success() {
            return Ok(());
        }
        Err(BenchError::Engine(format!(
            "energy: `sudo -n` is not permitted (powermetrics needs \
             root): {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }

    /// A running `powermetrics` subprocess metering the window.
    #[derive(Debug)]
    pub struct Run {
        child: Option<Child>,
        /// Instant the process spawned — origin of the sample
        /// timeline, before the measured window opens.
        spawned_at: Instant,
        /// The requested `-i` sample interval.
        interval: Duration,
        /// Complete plist documents as `powermetrics` emits them.
        samples_rx: Receiver<Vec<u8>>,
        /// Documents taken off the channel before the consumer runs
        /// — `spawn` receives the first one to prove startup —
        /// replayed in arrival order ahead of `samples_rx`.
        pending: VecDeque<Vec<u8>>,
        /// Drains stdout, splitting the NUL-separated stream into
        /// documents.
        out: Option<std::thread::JoinHandle<()>>,
        /// Drains stderr verbatim.
        err: Option<std::thread::JoinHandle<Vec<u8>>>,
    }

    /// Drains the pipe, splitting `powermetrics`' NUL-separated plist
    /// stream into documents delivered over `tx`; the truncated tail
    /// after a kill is sent too and the parser skips it.
    fn doc_reader(
        pipe: Option<impl Read + Send + 'static>,
        tx: Sender<Vec<u8>>,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            let Some(mut pipe) = pipe else {
                return;
            };
            let mut buf = Vec::new();
            let mut chunk = [0_u8; 16 * 1024];
            loop {
                match pipe.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                }
                while let Some(pos) = buf.iter().position(|&b| b == 0) {
                    let doc: Vec<u8> = buf.drain(..pos).collect();
                    buf.remove(0);
                    if !doc.is_empty() && tx.send(doc).is_err() {
                        return;
                    }
                }
            }
            if !buf.is_empty() {
                let _ = tx.send(buf);
            }
        })
    }

    fn reader(pipe: Option<impl Read + Send + 'static>) -> std::thread::JoinHandle<Vec<u8>> {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut buf);
            }
            buf
        })
    }

    /// SIGTERMs the `sudo` child — `sudo` forwards the signal to the
    /// root-owned `powermetrics`, which flushes the current sample and
    /// exits. A bare SIGKILL would orphan `powermetrics` holding the
    /// stdout pipe open and the reader threads would never finish.
    #[cfg(unix)]
    fn terminate(child: &Child) {
        if let Ok(pid) = i32::try_from(child.id()) {
            // SAFETY: a signal send; ESRCH on an exited child is
            // harmless.
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
    }

    /// Non-unix fallback: no `sudo`/`powermetrics` there anyway.
    #[cfg(not(unix))]
    fn terminate(child: &mut Child) {
        let _ = child.kill();
    }

    impl Drop for Run {
        /// An error between `begin` and `finish` must not leak a
        /// rooted `powermetrics`.
        fn drop(&mut self) {
            let _ = self.stop();
        }
    }

    /// How long the child has to exit on SIGTERM before the sampler
    /// is force-stopped.
    const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

    /// Startup failure bound in sample intervals: the spawn fails
    /// when the meter's first document has not arrived after
    /// `interval * STARTUP_INTERVALS`. A bound on how long startup
    /// may take, not a guess at when the meter becomes ready —
    /// readiness is proven by the document itself.
    const STARTUP_INTERVALS: u32 = 10;

    impl Run {
        /// Wraps an already-spawned child: the `powermetrics` spawn and
        /// the stand-ins the tests drive share this path so both get
        /// the reader threads and the terminating [`Drop`].
        ///
        /// `spawned_at` anchors the sample timeline; the caller
        /// captures it at the spawn call, before the reader threads
        /// start.
        pub(crate) fn from_child(child: Child, interval: Duration, spawned_at: Instant) -> Self {
            let (tx, samples_rx) = std::sync::mpsc::channel();
            let mut child = child;
            let out = doc_reader(child.stdout.take(), tx);
            let err = reader(child.stderr.take());
            Self {
                child: Some(child),
                spawned_at,
                interval,
                samples_rx,
                pending: VecDeque::new(),
                out: Some(out),
                err: Some(err),
            }
        }

        /// Spawns `sudo -n powermetrics --samplers cpu_power,gpu_power
        /// -i <ms> --format plist` for the window about to open.
        /// `window_hint` picks the sample interval (~1/10 of the
        /// window, clamped to 100–1000 ms).
        ///
        /// # Errors
        /// [`BenchError::Engine`] when the spawn fails or `sudo -n`
        /// isn't permitted (the process exits immediately).
        pub fn spawn(window_hint: Duration) -> Result<Self, BenchError> {
            let interval_ms = (window_hint.as_millis() / 10).clamp(100, 1000);
            let spawned_at = Instant::now();
            let child = Command::new("sudo")
                .arg("-n")
                .arg("powermetrics")
                .arg("--samplers")
                .arg("cpu_power,gpu_power")
                .arg("-i")
                .arg(interval_ms.to_string())
                .arg("--format")
                .arg("plist")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| {
                    BenchError::Engine(format!("energy: cannot spawn `sudo -n powermetrics`: {e}"))
                })?;
            let mut run = Self::from_child(
                child,
                Duration::from_millis(u64::try_from(interval_ms).unwrap_or(1000)),
                spawned_at,
            );
            // Startup is proven by the first document: one arriving
            // means the meter is sampling, while `sudo -n` refusing
            // or `powermetrics` dying early ends the stream. `bound`
            // is the failure bound on how long that may take — not a
            // readiness guess.
            let bound = run.interval.saturating_mul(STARTUP_INTERVALS);
            match run.samples_rx.recv_timeout(bound) {
                Ok(doc) => run.pending.push_back(doc),
                Err(RecvTimeoutError::Disconnected) => {
                    let stderr = run
                        .err
                        .take()
                        .and_then(|h| h.join().ok())
                        .unwrap_or_default();
                    let stderr = String::from_utf8_lossy(&stderr).trim().to_owned();
                    return Err(BenchError::Engine(format!(
                        "energy: `sudo -n powermetrics` exited before the measured window: {stderr}"
                    )));
                }
                Err(RecvTimeoutError::Timeout) => {
                    // Dropping `run` SIGTERMs the child and reaps it
                    // within `SHUTDOWN_TIMEOUT`.
                    return Err(BenchError::Engine(format!(
                        "energy: powermetrics produced no sample within {bound:?}"
                    )));
                }
            }
            Ok(run)
        }

        /// The next document: `pending` first, in arrival order, then
        /// the channel — a document `spawn` received while proving
        /// startup belongs on the timeline exactly as if it had
        /// arrived now.
        fn recv_doc(&mut self, timeout: Duration) -> Result<Vec<u8>, RecvTimeoutError> {
            if let Some(doc) = self.pending.pop_front() {
                return Ok(doc);
            }
            self.samples_rx.recv_timeout(timeout)
        }

        /// Non-blocking counterpart of [`Run::recv_doc`]; `None` when
        /// neither `pending` nor the channel holds a document.
        fn try_recv_doc(&mut self) -> Option<Vec<u8>> {
            self.pending
                .pop_front()
                .or_else(|| self.samples_rx.try_recv().ok())
        }

        /// The child pid — the tests use it to prove termination.
        #[cfg(all(test, unix))]
        pub(crate) fn pid(&self) -> u32 {
            self.child.as_ref().map_or(0, Child::id)
        }

        /// Whether the received samples cover through `end` — a sample
        /// whose interval boundary lands past `end` makes the tail
        /// sample complete.
        fn covered(&self, samples: &[Sample], end: Instant) -> bool {
            samples
                .last()
                .is_some_and(|s| self.spawned_at + s.end >= end)
        }

        /// Receives documents until a sample whose interval boundary
        /// lands past `end` — the tail sample then covers the window's
        /// close.
        ///
        /// # Errors
        /// [`BenchError::Engine`] when `powermetrics` exits before
        /// covering the window, a document fails to decode (its
        /// `elapsed_ns` is lost and with it the timeline), or no
        /// covering sample arrives within a few intervals of `end`.
        /// Partial coverage is never returned.
        fn collect_samples(&mut self, end: Instant) -> Result<Vec<Sample>, BenchError> {
            let mut offset = Duration::ZERO;
            let mut samples = Vec::new();
            // The boundary sample lands within about an interval of
            // `end`; a dead or hung `powermetrics` must not hang the
            // bench either.
            let deadline = end + self.interval.saturating_mul(4).max(Duration::from_secs(2));
            while !self.covered(&samples, end) {
                let Some(timeout) = deadline.checked_duration_since(Instant::now()) else {
                    return Err(BenchError::Engine(
                        "energy: powermetrics produced no sample past the window's end".into(),
                    ));
                };
                match self.recv_doc(timeout) {
                    Ok(chunk) => {
                        let Some(sample) = parse_doc(&chunk, offset) else {
                            return Err(BenchError::Engine(format!(
                                "energy: could not decode a powermetrics sample ({} bytes)",
                                chunk.len()
                            )));
                        };
                        offset = sample.end;
                        samples.push(sample);
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        return Err(BenchError::Engine(
                            "energy: timed out waiting for a powermetrics sample".into(),
                        ));
                    }
                    Err(RecvTimeoutError::Disconnected) => {
                        return Err(BenchError::Engine(
                            "energy: powermetrics exited before covering the window".into(),
                        ));
                    }
                }
            }
            Ok(samples)
        }

        /// SIGTERMs the child and reaps it within
        /// [`SHUTDOWN_TIMEOUT`]; on expiry this run's `powermetrics` —
        /// the child of this run's `sudo` — is signalled through `sudo
        /// pkill -P` first: reaping `sudo` alone orphans the sampler,
        /// which keeps the stdout pipe open, while a blanket `pkill
        /// powermetrics` would hit samplers this run did not start.
        /// The child is then reaped.
        ///
        /// # Errors
        /// [`BenchError::Engine`] when the child survived SIGTERM past
        /// the deadline and had to be force-stopped; the message says
        /// whether the scoped signal reached the sampler.
        fn stop(&mut self) -> Result<(), BenchError> {
            let Some(mut child) = self.child.take() else {
                return Ok(());
            };
            #[cfg(unix)]
            terminate(&child);
            #[cfg(not(unix))]
            terminate(&mut child);
            let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
            let alive = loop {
                match child.try_wait() {
                    Ok(Some(_)) => break false,
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Ok(None) | Err(_) => break true,
                }
            };
            if !alive {
                return Ok(());
            }
            // The sampler is root-owned: only `sudo` can signal it,
            // and only the sampler under this run's `sudo`.
            #[cfg(unix)]
            let detail = {
                let pid = i32::try_from(child.id()).unwrap_or(i32::MAX);
                let detail = match Command::new("sudo")
                    .args(["-n", "pkill", "-TERM", "-P"])
                    .arg(pid.to_string())
                    .args(["-x", "powermetrics"])
                    .output()
                {
                    Ok(out) if out.status.success() => {
                        "the sampler was signalled directly".to_owned()
                    }
                    Ok(out) => {
                        let stderr = String::from_utf8_lossy(&out.stderr);
                        let stderr = stderr.trim();
                        format!(
                            "the scoped pkill of the sampler failed ({}){}",
                            out.status,
                            if stderr.is_empty() {
                                String::new()
                            } else {
                                format!(": {stderr}")
                            }
                        )
                    }
                    Err(e) => format!("could not run `sudo -n pkill` for the sampler: {e}"),
                };
                // SAFETY: a signal send; ESRCH on an exited child is
                // harmless.
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
                detail
            };
            #[cfg(not(unix))]
            let detail = "SIGKILLed".to_owned();
            let _ = child.wait();
            Err(BenchError::Engine(format!(
                "energy: powermetrics did not exit on SIGTERM; {detail}"
            )))
        }

        /// Stops the subprocess after the window's tail sample and
        /// integrates the window-clipped samples into a report.
        ///
        /// `start`/`end` are the measured window's bounds.
        ///
        /// # Errors
        /// [`BenchError::Engine`] when `powermetrics` did not cover the
        /// window, produced no usable samples, or had to be
        /// force-stopped.
        pub fn finish(
            mut self,
            start: Instant,
            end: Instant,
            frames: u32,
        ) -> Result<Outcome, BenchError> {
            let mut samples = self.collect_samples(end)?;
            let stop_err = self.stop();
            // The sampler flushes a final document on SIGTERM; the
            // reader finishes on pipe EOF, then anything it delivered
            // is folded onto the timeline before integrating.
            if let Some(out) = self.out.take() {
                let _ = out.join();
            }
            let mut offset = samples.last().map_or(Duration::ZERO, |s| s.end);
            while let Some(chunk) = self.try_recv_doc() {
                if let Some(sample) = parse_doc(&chunk, offset) {
                    offset = sample.end;
                    samples.push(sample);
                }
            }
            let stderr = self
                .err
                .take()
                .and_then(|h| h.join().ok())
                .unwrap_or_default();
            let stderr = String::from_utf8_lossy(&stderr).trim().to_owned();
            stop_err?;
            if samples.is_empty() {
                return Err(BenchError::Engine(format!(
                    "energy: powermetrics produced no samples over the measured window{}",
                    if stderr.is_empty() {
                        String::new()
                    } else {
                        format!(": {stderr}")
                    }
                )));
            }
            // A machine whose GPU and CPU expose no energy counters
            // (a paravirtual VM) still emits samples, with no
            // processor section: no energy was measured, and a zero
            // report would read as a measurement.
            if samples
                .iter()
                .all(|s| s.joules.is_empty() && s.package.is_none())
            {
                return Err(BenchError::Engine(
                    "energy: powermetrics reported no energy counters on this machine".into(),
                ));
            }
            let window = (
                start.saturating_duration_since(self.spawned_at),
                end.saturating_duration_since(self.spawned_at),
            );
            let attributed = attribute(&samples, window.0, window.1);
            let window_s = end.saturating_duration_since(start).as_secs_f64();
            Ok(Outcome {
                report: report(&attributed, window_s, frames),
                thermal_pressure: attributed.thermal_pressure,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    /// Real `enabled_rails` content as produced by the Pixel kernel —
    /// `CH<idx>[<RAIL>]:<subsystem>` per line.
    const ENABLED_RAILS: &str = "\
CH0[S10M_VDD_TPU]:Tensor
CH1[VSYS_PWR_MODEM]:Cellular
CH2[VSYS_PWR_RFFE]:Cellular
CH3[S2M_VDD_CPUCL2]:CPU
CH4[S3M_VDD_CPUCL1]:CPU
CH5[S4M_VDD_CPUCL0]:CPU
CH6[S5M_VDD_INT]:SoC
CH7[S1M_VDD_MIF]:Memory
";

    /// Real `energy_value` content (captured from a Pixel 6, values
    /// simplified): a `t=<ms>` timestamp line then one
    /// `CH<idx>(T=<ms>)[<RAIL>], <µWs>` line per channel.
    const ENERGY_VALUE: &str = "\
t=349894
CH0(T=349894)[S10M_VDD_TPU], 5578756
CH1(T=349894)[VSYS_PWR_MODEM], 29110940
CH2(T=349894)[VSYS_PWR_RFFE], 3166046
CH3(T=349894)[S2M_VDD_CPUCL2], 30203502
CH4(T=349894)[S3M_VDD_CPUCL1], 23377533
CH5(T=349894)[S4M_VDD_CPUCL0], 46356942
CH6(T=349894)[S5M_VDD_INT], 10771876
CH7(T=349894)[S1M_VDD_MIF], 21091363
";

    #[test]
    fn enabled_rails_parses() {
        let rows = odpm::parse_enabled_rails(ENABLED_RAILS);
        assert_eq!(rows.len(), 8);
        assert_eq!(rows[0], (0, "S10M_VDD_TPU".into(), "Tensor".into()));
        assert_eq!(rows[3], (3, "S2M_VDD_CPUCL2".into(), "CPU".into()));
    }

    #[test]
    fn enabled_rails_skips_malformed_lines() {
        let rows = odpm::parse_enabled_rails("garbage\nCH2[VSYS_PWR_RFFE]:Cellular\n");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1, "VSYS_PWR_RFFE");
    }

    #[test]
    fn energy_value_parses() {
        let value = odpm::parse_energy_value(ENERGY_VALUE).unwrap();
        assert_eq!(value.timestamp_ms, 349_894);
        assert_eq!(value.rows.len(), 8);
        assert_eq!(value.rows[3], (3, "S2M_VDD_CPUCL2".into(), 30_203_502));
        assert_eq!(value.rows[7], (7, "S1M_VDD_MIF".into(), 21_091_363));
    }

    #[test]
    fn energy_value_rejects_bad_timestamp() {
        let err = odpm::parse_energy_value("nope\nCH0(T=1)[X], 1\n").unwrap_err();
        assert!(err.to_string().contains("malformed"), "{err}");
    }

    #[test]
    fn energy_value_rejects_bad_row() {
        let err = odpm::parse_energy_value("t=1\nCH0 T=1 [X], 1\n").unwrap_err();
        assert!(err.to_string().contains("malformed"), "{err}");
    }

    /// Writes a stand-in `iio:device*` directory. The colon in sysfs device
    /// names is not a valid path character on Windows, so the fixtures only
    /// build on Unix, where the real ODPM tree lives.
    #[cfg(unix)]
    fn fake_device(root: &std::path::Path, name: &str, rails: &str, energy: &str) {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("enabled_rails"), rails).unwrap();
        std::fs::write(dir.join("energy_value"), energy).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn snapshot_reads_all_devices() {
        let root = std::env::temp_dir().join(format!("odpm-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        fake_device(
            &root,
            "iio:device0",
            "CH3[S2M_VDD_CPUCL2]:CPU\n",
            "t=349894\nCH3(T=349894)[S2M_VDD_CPUCL2], 30203502\n",
        );
        fake_device(
            &root,
            "iio:device1",
            "CH6[S2S_VDD_G3D]:GPU\nCH7[VSYS_PWR_DISPLAY]:Display\n",
            "t=359458\nCH6(T=359458)[S2S_VDD_G3D], 5315420\nCH7(T=359458)[VSYS_PWR_DISPLAY], 81221665\n",
        );
        let snap = odpm::snapshot_at(&root).unwrap();
        assert_eq!(snap.rails.len(), 3);
        assert_eq!(snap.rails["S2M_VDD_CPUCL2"], 30_203_502);
        assert_eq!(snap.subsystems["VSYS_PWR_DISPLAY"], "Display");
        assert_eq!(snap.timestamp_ms, Some(359_458));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn snapshot_missing_root_names_path() {
        let err = odpm::snapshot_at(std::path::Path::new("/definitely/no/odpm/here")).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("/definitely/no/odpm/here"), "{msg}");
        assert!(msg.contains("energy:"), "{msg}");
    }

    #[test]
    fn snapshot_without_devices_errors() {
        let root = std::env::temp_dir().join(format!("odpm-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let err = odpm::snapshot_at(&root).unwrap_err();
        assert!(err.to_string().contains("no ODPM"), "{err}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn diff_report_integrates() {
        let before = odpm::Snapshot {
            rails: BTreeMap::from([("CPU".into(), 1_000_000u64), ("GPU".into(), 500_000)]),
            subsystems: BTreeMap::from([
                ("CPU".into(), "CPU".into()),
                ("GPU".into(), "GPU".into()),
            ]),
            timestamp_ms: Some(100_000),
        };
        let after = odpm::Snapshot {
            rails: BTreeMap::from([("CPU".into(), 3_000_000u64), ("GPU".into(), 700_000)]),
            subsystems: BTreeMap::from([
                ("CPU".into(), "CPU".into()),
                ("GPU".into(), "GPU".into()),
            ]),
            timestamp_ms: Some(102_000),
        };
        let report = odpm::diff_report(&before, &after, Duration::from_secs(2), 40).unwrap();
        // 2 J over 40 frames / 2 s: 0.05 J/frame, 1 W.
        let cpu = &report.rails["CPU"];
        assert_eq!(cpu.joules, 2.0);
        assert_eq!(cpu.joules_per_frame, 0.05);
        assert_eq!(cpu.watts, 1.0);
        assert_eq!(report.total_joules, 2.2);
    }

    #[test]
    fn diff_report_rejects_a_changed_rail_set() {
        let before = odpm::Snapshot {
            rails: BTreeMap::from([("CPU".into(), 1_000_000u64)]),
            subsystems: BTreeMap::new(),
            timestamp_ms: Some(100_000),
        };
        let after = odpm::Snapshot {
            rails: BTreeMap::from([("CPU".into(), 3_000_000u64), ("GPU".into(), 700_000)]),
            subsystems: BTreeMap::new(),
            timestamp_ms: Some(102_000),
        };
        let err = odpm::diff_report(&before, &after, Duration::from_secs(2), 40).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("appeared"), "{msg}");
        assert!(msg.contains("GPU"), "{msg}");

        // A rail present before but gone after is also a mismatch.
        let err = odpm::diff_report(&after, &before, Duration::from_secs(2), 40).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("disappeared"), "{msg}");
        assert!(msg.contains("GPU"), "{msg}");
    }

    /// A `powermetrics --format plist` sample on Apple silicon —
    /// one NUL-separated XML plist per sample.
    const SAMPLE_PLIST: &str = "\
<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">
<plist version=\"1.0\">
<dict>
\t<key>elapsed_ns</key>
\t<integer>500000000</integer>
\t<key>hw_model</key>
\t<string>J413AP</string>
\t<key>thermal_pressure</key>
\t<string>Nominal</string>
\t<key>processor</key>
\t<dict>
\t\t<key>clusters</key>
\t\t<array>
\t\t\t<dict>
\t\t\t\t<key>name</key>
\t\t\t\t<string>E-Cluster</string>
\t\t\t\t<key>freq_hz</key>
\t\t\t\t<real>9.6e+08</real>
\t\t\t\t<key>idle_ratio</key>
\t\t\t\t<real>0.9</real>
\t\t\t\t<key>dvfm_states</key>
\t\t\t\t<array/>
\t\t\t\t<key>cpus</key>
\t\t\t\t<array/>
\t\t\t</dict>
\t\t</array>
\t\t<key>ane_energy</key>
\t\t<integer>12</integer>
\t\t<key>cpu_energy</key>
\t\t<integer>640</integer>
\t\t<key>gpu_energy</key>
\t\t<integer>305</integer>
\t\t<key>dram_energy</key>
\t\t<integer>89</integer>
\t\t<key>package_energy</key>
\t\t<integer>1046</integer>
\t\t<key>combined_power</key>
\t\t<real>2092.0</real>
\t</dict>
\t<key>gpu</key>
\t<dict>
\t\t<key>idle_ratio</key>
\t\t<real>0.7</real>
\t\t<key>freq_hz</key>
\t\t<real>4.0e+08</real>
\t\t<key>dvfm_states</key>
\t\t<array/>
\t</dict>
</dict>
</plist>
";

    #[test]
    fn powermetrics_parses_nul_separated_samples() {
        let mut stream = SAMPLE_PLIST.as_bytes().to_vec();
        stream.push(0);
        stream.extend_from_slice(SAMPLE_PLIST.as_bytes());
        let samples = powermetrics::parse_stream(&stream);
        assert_eq!(samples.len(), 2);
        // Each sample covers 500 ms; the timeline accumulates.
        assert_eq!(samples[0].start, Duration::ZERO);
        assert_eq!(samples[0].end, Duration::from_millis(500));
        assert_eq!(samples[1].start, Duration::from_millis(500));
        assert_eq!(samples[1].end, Duration::from_secs(1));
        assert_eq!(samples[0].joules["cpu"], 0.64);
        assert_eq!(samples[0].joules["gpu"], 0.305);
        assert_eq!(samples[0].joules["ane"], 0.012);
        assert_eq!(samples[0].joules["dram"], 0.089);
        // The package counter is kept beside the domains, never as a
        // rail of its own.
        assert_eq!(samples[0].package, Some(1.046));
        assert_eq!(samples[1].thermal_pressure.as_deref(), Some("Nominal"));
    }

    #[test]
    fn powermetrics_skips_truncated_tail() {
        let mut stream = SAMPLE_PLIST.as_bytes().to_vec();
        stream.push(0);
        stream.extend_from_slice(&SAMPLE_PLIST.as_bytes()[..100]);
        let samples = powermetrics::parse_stream(&stream);
        assert_eq!(samples.len(), 1);
    }

    #[test]
    fn powermetrics_intel_package_joules_maps_to_cpu() {
        let plist = "\
<?xml version=\"1.0\"?>
<plist version=\"1.0\">
<dict>
\t<key>elapsed_ns</key><integer>1000000000</integer>
\t<key>processor</key>
\t<dict>
\t\t<key>package_joules</key><real>2.5</real>
\t</dict>
</dict>
</plist>
";
        let samples = powermetrics::parse_stream(plist.as_bytes());
        assert_eq!(samples.len(), 1);
        // The only Intel counter feeds the cpu rail and the package
        // total alike — the report's total is not doubled.
        assert_eq!(samples[0].joules["cpu"], 2.5);
        assert_eq!(samples[0].package, Some(2.5));
    }

    /// A synthetic [`powermetrics::Sample`] spanning
    /// `[start_ms, end_ms]` with `cpu` joules only.
    fn pm_sample(
        start_ms: u64,
        end_ms: u64,
        cpu: f64,
        package: Option<f64>,
    ) -> powermetrics::Sample {
        powermetrics::Sample {
            start: Duration::from_millis(start_ms),
            end: Duration::from_millis(end_ms),
            joules: BTreeMap::from([("cpu".into(), cpu)]),
            package,
            thermal_pressure: None,
        }
    }

    #[test]
    fn attribute_scales_boundary_samples_by_overlap() {
        // Samples cover [0,20] and [20,40]; the window is [10,30]:
        // half of each interval falls inside.
        let samples = [
            pm_sample(0, 20, 2.0, Some(4.0)),
            pm_sample(20, 40, 2.0, Some(4.0)),
        ];
        let a = powermetrics::attribute(
            &samples,
            Duration::from_millis(10),
            Duration::from_millis(30),
        );
        assert_eq!(a.joules["cpu"], 2.0);
        assert_eq!(a.package, Some(4.0));
    }

    #[test]
    fn attribute_drops_samples_outside_the_window() {
        let samples = [
            pm_sample(0, 20, 4.0, Some(8.0)),
            pm_sample(20, 40, 4.0, Some(8.0)),
            pm_sample(40, 60, 4.0, Some(8.0)),
        ];
        // Only the middle sample overlaps the window.
        let a = powermetrics::attribute(
            &samples,
            Duration::from_millis(22),
            Duration::from_millis(38),
        );
        assert_eq!(a.joules["cpu"], 3.2);
        assert_eq!(a.package, Some(6.4));
    }

    /// Spawns a stand-in child (`bash -c <args>`) in place of
    /// `sudo -n powermetrics`.
    #[cfg(unix)]
    fn stand_in(args: &[&str]) -> std::process::Child {
        use std::process::{Command, Stdio};
        Command::new("bash")
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn stand-in")
    }

    #[cfg(unix)]
    fn stand_in_run(args: &[&str]) -> powermetrics::Run {
        powermetrics::Run::from_child(stand_in(args), Duration::from_millis(50), Instant::now())
    }

    #[cfg(unix)]
    fn process_alive(pid: u32) -> bool {
        // SAFETY: signal 0 probes existence without delivering.
        unsafe { libc::kill(i32::try_from(pid).unwrap_or(i32::MAX), 0) == 0 }
    }

    #[test]
    #[cfg(unix)]
    fn finish_stops_the_child_process() {
        let run = stand_in_run(&["-c", "sleep 60"]);
        let pid = run.pid();
        let end = Instant::now() + Duration::from_millis(10);
        // `finish` errors — `sleep` produces no samples — but must
        // still stop the child.
        let _ = run.finish(Instant::now(), end, 1);
        assert!(!process_alive(pid), "child survived finish");
    }

    #[test]
    #[cfg(unix)]
    fn finish_errors_when_the_stream_ends_before_the_window() {
        // The stand-in exits at once: premature EOF must be an
        // error, not a partial report.
        let run = stand_in_run(&["-c", "exit 0"]);
        let err = run
            .finish(Instant::now(), Instant::now() + Duration::from_secs(30), 1)
            .unwrap_err();
        assert!(err.to_string().contains("exited before covering"), "{err}");
    }

    #[test]
    #[cfg(unix)]
    fn finish_errors_when_a_sample_does_not_decode() {
        // One undecodable document loses the elapsed_ns timeline.
        let run = stand_in_run(&["-c", "printf 'x\\0'; sleep 60"]);
        let pid = run.pid();
        let err = run
            .finish(
                Instant::now(),
                Instant::now() + Duration::from_millis(10),
                1,
            )
            .unwrap_err();
        assert!(err.to_string().contains("could not decode"), "{err}");
        assert!(!process_alive(pid), "child survived finish");
    }

    #[test]
    #[cfg(unix)]
    fn drop_stops_the_child_process() {
        let run = stand_in_run(&["-c", "sleep 60"]);
        let pid = run.pid();
        drop(run);
        assert!(!process_alive(pid), "child survived drop");
    }

    #[test]
    #[cfg(unix)]
    fn drop_bounds_shutdown_of_a_term_ignoring_child() {
        // The stand-in ignores SIGTERM; shutdown must still be
        // bounded and the child must not survive. `Run` owns the
        // child's pipes, so the stand-in proves its trap is
        // installed through a marker file written only after `trap`
        // runs — the wait below is on that condition, not a delay.
        let markers = [
            std::env::temp_dir().join(format!("energy-term-trap-{}-run", std::process::id())),
            std::env::temp_dir().join(format!("energy-term-trap-{}-bystander", std::process::id())),
        ];
        for marker in &markers {
            let _ = std::fs::remove_file(marker);
        }
        let script = "trap '' TERM; : > \"$0\"; exec sleep 60";
        let run = stand_in_run(&[
            "-c",
            script,
            markers[0].to_str().expect("marker path is UTF-8"),
        ]);
        let pid = run.pid();
        // An identical stand-in not parented by this run: only the
        // child of this run's own process may be signalled.
        let mut bystander = stand_in(&[
            "-c",
            script,
            markers[1].to_str().expect("marker path is UTF-8"),
        ]);
        let bystander_pid = bystander.id();
        let traps_deadline = Instant::now() + Duration::from_secs(10);
        for marker in &markers {
            while !marker.exists() {
                assert!(
                    Instant::now() < traps_deadline,
                    "stand-in did not install its TERM trap within 10 s: {}",
                    marker.display()
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        let started = Instant::now();
        drop(run);
        let elapsed = started.elapsed();
        // Took the force-stop path: outlasted the ~3 s SIGTERM
        // grace period yet came back bounded.
        assert!(
            elapsed >= Duration::from_secs(2),
            "shutdown ended before the SIGTERM grace period: {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_secs(15),
            "shutdown not bounded: {elapsed:?}"
        );
        assert!(!process_alive(pid), "SIGTERM-ignoring child survived drop");
        assert!(
            process_alive(bystander_pid),
            "shutdown signalled an unrelated process"
        );
        let _ = bystander.kill();
        let _ = bystander.wait();
        for marker in &markers {
            let _ = std::fs::remove_file(marker);
        }
    }
}
