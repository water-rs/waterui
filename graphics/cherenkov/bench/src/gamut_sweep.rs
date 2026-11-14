//! `gamut-sweep`: the #96 quality measurement.
//!
//! Sweeps the sRGB gamut boundary densely — every face of the linear P3
//! cube plus lines from the grey axis out to all six P3 primaries and
//! secondaries — and reports, per candidate, `ΔE_OK` against the CSS
//! Color 4 reference implemented below (candidate (a), the spec
//! algorithm, which measures itself at zero) and hue shift against the
//! input's hue. The clip row is the pre-#96 baseline.

use std::fmt::Write as _;
use std::path::Path;

use cherenkov_oracle::color::linear_p3_to_linear_srgb;
use cherenkov_oracle::gamut::{
    Anchor, delta_e_ok, gamut_map_srgb_project, linear_srgb_to_oklab, oklab_to_linear_srgb,
};

use crate::BenchError;

/// Candidate (a): CSS Color 4 §14.2.2 gamut mapping — `OKLCh` chroma
/// binary search with the local-MINDE rule (`JND` = 0.02 `ΔE_OK`,
/// epsilon = 1e-4). The spec algorithm, kept here as the sweep's
/// measurement reference only; the shipped map is
/// [`cherenkov_oracle::gamut::gamut_map_srgb_analytic`].
#[allow(clippy::many_single_char_names, clippy::while_float)] // names and the float loop mirror the spec pseudocode
fn gamut_map_srgb_css(rgb: [f64; 3]) -> [f64; 3] {
    const JND: f64 = 0.02;
    const EPSILON: f64 = 0.0001;
    fn in_gamut(rgb: [f64; 3]) -> bool {
        rgb.iter().all(|&c| (0.0..=1.0).contains(&c))
    }
    fn clip(rgb: [f64; 3]) -> [f64; 3] {
        rgb.map(|c| c.clamp(0.0, 1.0))
    }
    let lab = linear_srgb_to_oklab(rgb);
    let [l, a, b] = lab;
    if l >= 1.0 {
        return [1.0; 3];
    }
    if l <= 0.0 {
        return [0.0; 3];
    }
    let c0 = a.hypot(b);
    let h = b.atan2(a);
    let mut clipped = clip(rgb);
    let mut e = delta_e_ok(linear_srgb_to_oklab(clipped), lab);
    if e < JND {
        return clipped;
    }
    let (mut min, mut max) = (0.0, c0);
    let mut min_in_gamut = true;
    while max - min > EPSILON {
        let chroma = min.midpoint(max);
        let cur_lab = [l, chroma * h.cos(), chroma * h.sin()];
        let current = oklab_to_linear_srgb(cur_lab);
        if min_in_gamut && in_gamut(current) {
            min = chroma;
            continue;
        }
        clipped = clip(current);
        e = delta_e_ok(linear_srgb_to_oklab(clipped), cur_lab);
        if e < JND {
            if JND - e < EPSILON {
                return clipped;
            }
            min_in_gamut = false;
            min = chroma;
        } else {
            max = chroma;
        }
    }
    clipped
}

fn hue_deg(lab: [f64; 3]) -> f64 {
    lab[2].atan2(lab[1]).to_degrees().rem_euclid(360.0)
}

fn hue_diff_deg(a: [f64; 3], b: [f64; 3]) -> f64 {
    let d = (hue_deg(a) - hue_deg(b)).abs().rem_euclid(360.0);
    d.min(360.0 - d)
}

struct Bucket {
    de: Vec<f64>,
    hue_in: Vec<f64>,
    worst: (f64, [f64; 3], [f64; 3], [f64; 3]),
}

impl Bucket {
    const fn new() -> Self {
        Self {
            de: Vec::new(),
            hue_in: Vec::new(),
            worst: (0.0, [0.0; 3], [0.0; 3], [0.0; 3]),
        }
    }

    fn push(&mut self, input: [f64; 3], mapped: [f64; 3], reference: [f64; 3]) {
        let de = delta_e_ok(
            linear_srgb_to_oklab(mapped),
            linear_srgb_to_oklab(reference),
        );
        self.de.push(de);
        self.hue_in.push(hue_diff_deg(
            linear_srgb_to_oklab(mapped),
            linear_srgb_to_oklab(input),
        ));
        if de > self.worst.0 {
            self.worst = (de, input, mapped, reference);
        }
    }

    #[expect(
        clippy::cast_precision_loss,
        reason = "sample counts are far under 2^52"
    )]
    fn line(&self, name: &str, out: &mut String) {
        let mut de = self.de.clone();
        let mut hi = self.hue_in.clone();
        de.sort_by(f64::total_cmp);
        hi.sort_by(f64::total_cmp);
        let n = de.len();
        let q = |v: &[f64], p: usize| v[(n * p / 100).min(n - 1)];
        let _ = writeln!(
            out,
            "{name:>12}: n={n}  ΔE mean {:.5}  p50 {:.5}  p99 {:.5}  max {:.5}  |  hue° vs input p50 {:.4} p99 {:.4} max {:.4}",
            de.iter().sum::<f64>() / n as f64,
            q(&de, 50),
            q(&de, 99),
            de[n - 1],
            q(&hi, 50),
            q(&hi, 99),
            hi[n - 1],
        );
        let _ = writeln!(
            out,
            "{name:>12}  worst ΔE at srgb {:.3?} -> {:.3?} (reference {:.3?})",
            self.worst.1, self.worst.2, self.worst.3
        );
    }
}

/// Runs the sweep; returns the report text.
fn sweep() -> String {
    let mut samples: Vec<[f64; 3]> = Vec::new();
    // P3 cube surface: six faces on a 65×65 grid.
    let n = 65;
    for face in 0..6 {
        for i in 0..n {
            for j in 0..n {
                let u = f64::from(i) / f64::from(n - 1);
                let v = f64::from(j) / f64::from(n - 1);
                let fixed = f64::from(face % 2);
                let p = match face / 2 {
                    0 => [fixed, u, v],
                    1 => [u, fixed, v],
                    _ => [u, v, fixed],
                };
                samples.push(linear_p3_to_linear_srgb(p));
            }
        }
    }
    // Grey-axis -> P3 vertex lines, extended past the boundary.
    for g0 in [0.1, 0.25, 0.4, 0.5, 0.6, 0.75, 0.9] {
        for corner in [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 1.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 0.0],
        ] {
            for i in 0..200 {
                let t = f64::from(i) / 199.0 * 1.6;
                samples.push(linear_p3_to_linear_srgb([
                    t.mul_add(corner[0] - g0, g0),
                    t.mul_add(corner[1] - g0, g0),
                    t.mul_add(corner[2] - g0, g0),
                ]));
            }
        }
    }

    let mut clip = Bucket::new();
    let mut preserve = Bucket::new();
    let mut cusp = Bucket::new();
    let mut adaptive = Bucket::new();
    let mut n_out = 0usize;
    for srgb in samples {
        if srgb.iter().all(|&c| (0.0..=1.0).contains(&c)) {
            continue;
        }
        n_out += 1;
        let reference = gamut_map_srgb_css(srgb);
        clip.push(srgb, srgb.map(|c| c.clamp(0.0, 1.0)), reference);
        preserve.push(
            srgb,
            gamut_map_srgb_project(srgb, Anchor::Preserve),
            reference,
        );
        cusp.push(srgb, gamut_map_srgb_project(srgb, Anchor::Cusp), reference);
        adaptive.push(
            srgb,
            gamut_map_srgb_project(srgb, Anchor::Adaptive),
            reference,
        );
    }
    let mut out = String::new();
    let _ = writeln!(
        out,
        "gamut sweep: {n_out} out-of-gamut linear-sRGB samples \
         (P3 cube surface 65x65x6 + 7x6 grey-axis->vertex lines x200)"
    );
    clip.line("clip", &mut out);
    preserve.line("b preserve", &mut out);
    cusp.line("b l_cusp", &mut out);
    adaptive.line("b adaptive", &mut out);
    out
}

/// The `gamut-sweep` subcommand.
///
/// # Errors
/// `BenchError` on a failed `--out` write.
pub fn run(out: Option<&Path>) -> Result<(), BenchError> {
    let report = sweep();
    if let Some(path) = out {
        std::fs::write(path, &report)
            .map_err(|e| BenchError::Engine(format!("write {}: {e}", path.display())))?;
        tracing::info!(out = %path.display(), "gamut-sweep");
    } else {
        println!("{report}");
    }
    Ok(())
}
