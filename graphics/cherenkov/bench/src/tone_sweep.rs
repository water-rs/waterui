//! `tone-sweep`: the #97 curve measurement.
//!
//! Sweeps the presentation shoulder for each candidate over `0..16×` SDR
//! white at display headrooms 1, 2, 4, 8 and reports monotonicity, the
//! derivative on both sides of the `x = 1` knee (the `C1` check), the
//! largest derivative in the compressed range, the plateau onset (the
//! largest `x` still strictly increasing), and where 4×/8× inputs land.
//! A hue section verifies the per-pixel `max`-channel scalar preserves
//! channel ratios on saturated HDR colours.
//!
//! The shipped curve is [`cherenkov_oracle::tone`]; the BT.2390-adapted
//! EETF is kept here as the sweep's measurement reference only — the
//! shape choice is recorded in the #2 decision log.

use std::cmp::Ordering;
use std::fmt::Write as _;
use std::path::Path;

use crate::BenchError;

/// Candidate (b): the BT.2390 EETF adapted to relative headroom — a
/// Hermite cubic from `(1, 1, slope 1)` to `(3h−2, h, slope 0)`, flat at
/// `h` beyond. The spec's `KS = 1.5·maxLum − 0.5` knee rule sets the
/// source extent at `3h−2`. Measurement reference only; the shipped
/// shoulder is `cherenkov_oracle::tone::tone` (extended-Reinhard).
fn eetf(x: f64, h: f64) -> f64 {
    if x <= 1.0 {
        return x;
    }
    let d = (h - 1.0).max(0.0);
    let span = 3.0 * d;
    if span == 0.0 {
        return 1.0;
    }
    let u = 1.0 - (x - 1.0) / span;
    if u <= 0.0 {
        return h;
    }
    (d * u * u).mul_add(-u, h)
}

/// A tone-map shoulder `(x, headroom) -> y`.
type Shoulder = fn(f64, f64) -> f64;

/// The candidates: name and shoulder.
const CANDIDATES: &[(&str, Shoulder)] = &[
    ("bt2390-eetf", eetf),
    ("extended-reinhard", cherenkov_oracle::tone::tone),
];

/// Headrooms the sweep measures: SDR, HDR and wide displays.
const HEADROOMS: &[f64] = &[1.0, 2.0, 4.0, 8.0];

/// The HDR input ramp: `0..16×` SDR white at 64k samples.
const X_MAX: f64 = 16.0;
const STEPS: u32 = 65_536;

/// The per-pixel map one shoulder induces (`tone_map` with it).
fn pixel_map(shoulder: Shoulder, h: f64, rgb: [f64; 3]) -> [f64; 3] {
    let m = rgb[0].max(rgb[1]).max(rgb[2]);
    if m.partial_cmp(&1.0) != Some(Ordering::Greater) {
        return rgb;
    }
    let s = shoulder(m, h) / m;
    rgb.map(|c| c * s)
}

/// `tone-sweep`'s report — text on stdout, or written to `--out`.
pub(crate) fn run(out: Option<&Path>) -> Result<(), BenchError> {
    let mut report = String::new();
    let _ = writeln!(
        report,
        "monotonicity over 0..{X_MAX}x, knee derivative, plateau onset, output levels:"
    );
    let _ = writeln!(
        report,
        "{:<18} {:>5} {:>10} {:>10} {:>10} {:>10} {:>10} {:>8} {:>8}",
        "candidate", "H", "mono-viol", "f'(1-)", "f'(1+)", "max f'", "plateau@", "f(4x)", "f(8x)"
    );
    for &(name, shoulder) in CANDIDATES {
        for &h in HEADROOMS {
            let mut mono_viol = 0usize;
            let mut prev = shoulder(0.0, h);
            let mut max_deriv = 0.0f64;
            // The last strictly-increasing input before any plateau.
            let mut plateau_at = f64::INFINITY;
            for i in 1..=STEPS {
                let x = X_MAX * f64::from(i) / f64::from(STEPS);
                let y = shoulder(x, h);
                if y < prev {
                    mono_viol += 1;
                }
                if y.to_bits() == prev.to_bits() && plateau_at.is_infinite() && x > 1.0 {
                    plateau_at = x;
                }
                max_deriv = f64::max(max_deriv, (y - prev) / (X_MAX / f64::from(STEPS)));
                prev = y;
            }
            let eps = 1.0e-6;
            let dl = (shoulder(1.0, h) - shoulder(1.0 - eps, h)) / eps;
            let dr = (shoulder(1.0 + eps, h) - shoulder(1.0, h)) / eps;
            let plateau = if plateau_at.is_finite() {
                format!("{plateau_at:>10.3}")
            } else {
                format!("{:>10}", "never")
            };
            let _ = writeln!(
                report,
                "{name:<18} {h:>5.1} {mono_viol:>10} {dl:>10.6} {dr:>10.6} {max_deriv:>10.6} {plateau} {:>8.4} {:>8.4}",
                shoulder(4.0, h),
                shoulder(8.0, h),
            );
        }
    }

    let _ = writeln!(report);
    let _ = writeln!(
        report,
        "hue preservation — channel ratios through the per-pixel map:"
    );
    let _ = writeln!(
        report,
        "{:<18} {:>5} {:>30} {:>10} {:>10}",
        "candidate", "H", "tone_map(4,0.5,0.25)", "g/r in", "g/r out"
    );
    for &(name, shoulder) in CANDIDATES {
        for &h in HEADROOMS {
            let px = [4.0, 0.5, 0.25];
            let out_px = pixel_map(shoulder, h, px);
            let _ = writeln!(
                report,
                "{name:<18} {h:>5.1} {:>30} {:>10.6} {:>10.6}",
                format!("{out_px:?}"),
                px[1] / px[0],
                out_px[1] / out_px[0],
            );
        }
    }

    match out {
        Some(path) => {
            std::fs::write(path, &report)
                .map_err(|e| BenchError::Engine(format!("write {}: {e}", path.display())))?;
            tracing::info!(out = %path.display(), "tone-sweep");
        }
        None => print!("{report}"),
    }
    Ok(())
}
