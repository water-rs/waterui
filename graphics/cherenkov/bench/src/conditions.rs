//! Device conditions that contextualise a `measure` run.
//!
//! Thermal status, screen state and brightness — read by the binary
//! itself where the platform allows it. Everything here is
//! best-effort: a field is `null` when its source isn't readable
//! rather than guessed.
//!
//! - **Android**: `dumpsys thermalservice` for the thermal severity,
//!   `dumpsys power` (`mWakefulness=`) for screen state, `settings get
//!   system screen_brightness` for brightness. The zone temperature
//!   comes from `/sys/class/thermal` directly.
//! - **macOS**: `pmset -g powerstate IODisplayWrangler` for screen
//!   state; thermal status comes from `powermetrics` when `--energy`
//!   ran (its `thermal_pressure`).
//! - **Linux**: the thermal zone temperature only.

use crate::report::Conditions;

/// Collects the device's conditions at the end of a measured window.
///
/// `thermal_hint` supplies a thermal-status word already obtained from
/// the energy meter (macOS `powermetrics` `thermal_pressure`); the
/// platform's own source fills in when there is none.
#[must_use]
pub fn collect(thermal_hint: Option<String>) -> Conditions {
    Conditions {
        thermal_status: thermal_hint.or_else(thermal_status),
        thermal_celsius: crate::thermal_celsius(),
        screen_state: screen_state(),
        screen_brightness: screen_brightness(),
    }
}

/// Runs the first program that works, returning its stdout.
#[cfg(any(target_os = "android", target_vendor = "apple"))]
fn run(candidates: &[&str], args: &[&str]) -> Option<String> {
    for prog in candidates {
        if let Ok(out) = std::process::Command::new(prog).args(args).output()
            && out.status.success()
        {
            return String::from_utf8(out.stdout).ok();
        }
    }
    None
}

/// Android thermal severity from `dumpsys thermalservice`, which prints
/// either `Status{mOffline=0,mBootCompleted=true,mSeverity=N}` or a
/// `... Status: N` line.
#[cfg(target_os = "android")]
fn thermal_status() -> Option<String> {
    let out = run(&["dumpsys", "/system/bin/dumpsys"], &["thermalservice"])?;
    let severity = parse_thermal_severity(&out)?;
    Some(
        match severity {
            0 => "none",
            1 => "light",
            2 => "moderate",
            3 => "severe",
            4 => "critical",
            5 => "emergency",
            6 => "shutdown",
            n => return Some(format!("unknown:{n}")),
        }
        .to_owned(),
    )
}

#[cfg(target_os = "android")]
fn parse_thermal_severity(out: &str) -> Option<u32> {
    for line in out.lines() {
        if let Some(v) = line
            .split("mSeverity=")
            .nth(1)
            .and_then(|rest| rest.split([',', '}', ' ']).next())
            .and_then(|n| n.parse().ok())
        {
            return Some(v);
        }
        if let Some(v) = line
            .split("Status:")
            .nth(1)
            .and_then(|rest| rest.trim().parse().ok())
        {
            return Some(v);
        }
    }
    None
}

#[cfg(not(target_os = "android"))]
const fn thermal_status() -> Option<String> {
    None
}

/// Android screen state: `mWakefulness=Awake|Asleep|Dozing|Dreaming`
/// from `dumpsys power`, lowercased.
#[cfg(target_os = "android")]
fn screen_state() -> Option<String> {
    let out = run(&["dumpsys", "/system/bin/dumpsys"], &["power"])?;
    for line in out.lines() {
        let line = line.trim();
        if let Some(v) = line
            .split("mWakefulness=")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
        {
            return Some(v.to_lowercase());
        }
    }
    None
}

/// macOS screen state from `pmset -g powerstate IODisplayWrangler`:
/// the wrangler's power state, 0 = off, 4 = fully on.
#[cfg(target_vendor = "apple")]
fn screen_state() -> Option<String> {
    let out = run(&["pmset"], &["-g", "powerstate", "IODisplayWrangler"])?;
    for line in out.lines() {
        let mut parts = line.split_whitespace();
        if parts.next() == Some("IODisplayWrangler")
            && let Some(state) = parts.next().and_then(|v| v.parse::<u32>().ok())
        {
            return Some(
                match state {
                    0 => "off",
                    4 => "on",
                    _ => "dimmed",
                }
                .to_owned(),
            );
        }
    }
    None
}

/// Linux/other: no reliable unprivileged screen-state source.
#[cfg(not(any(target_os = "android", target_vendor = "apple")))]
const fn screen_state() -> Option<String> {
    None
}

/// Android brightness: `settings get system screen_brightness` (0–255),
/// falling back to the first readable `/sys/class/backlight/*/brightness`.
#[cfg(target_os = "android")]
fn screen_brightness() -> Option<u32> {
    if let Some(out) = run(
        &["settings", "/system/bin/settings"],
        &["get", "system", "screen_brightness"],
    ) && let Ok(v) = out.trim().parse()
    {
        return Some(v);
    }
    sysfs_brightness()
}

/// First readable `/sys/class/backlight/*/brightness`.
#[cfg(unix)]
fn sysfs_brightness() -> Option<u32> {
    let rd = std::fs::read_dir("/sys/class/backlight").ok()?;
    for entry in rd.flatten() {
        if let Ok(v) = std::fs::read_to_string(entry.path().join("brightness"))
            && let Ok(v) = v.trim().parse()
        {
            return Some(v);
        }
    }
    None
}

/// Non-Android unix: the backlight class alone, when it exists.
#[cfg(all(unix, not(target_os = "android")))]
fn screen_brightness() -> Option<u32> {
    sysfs_brightness()
}

#[cfg(not(unix))]
const fn screen_brightness() -> Option<u32> {
    None
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "android")]
    #[test]
    fn parses_thermal_severity() {
        assert_eq!(
            super::parse_thermal_severity("Status{mOffline=0,mBootCompleted=true,mSeverity=3}"),
            Some(3)
        );
        assert_eq!(
            super::parse_thermal_severity("Current Thermal Status: 1\n"),
            Some(1)
        );
        assert_eq!(super::parse_thermal_severity("nothing"), None);
    }
}
