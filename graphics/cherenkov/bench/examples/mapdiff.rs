//! Prints the adaptive and CSS f64 maps' outputs for hand-picked linear
//! sRGB inputs — a spot check of how far the two maps land apart (#158).

use cherenkov_oracle::gamut::{
    delta_e_ok, gamut_map_srgb_analytic, linear_srgb_to_oklab, oklab_to_linear_srgb,
};

#[allow(clippy::many_single_char_names, clippy::while_float)]
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

fn main() {
    for (name, srgb) in [
        ("p3 green", [-0.224_940_2, 1.042_056_9, -0.078_636_05]),
        ("p3 red", [1.224_940_2, -0.042_056_95, -0.019_637_55]),
        ("p3 blue", [-0.019_637_55, -0.078_636_05, 1.098_273_6]),
    ] {
        let a = gamut_map_srgb_analytic(srgb);
        let c = gamut_map_srgb_css(srgb);
        let de = delta_e_ok(linear_srgb_to_oklab(a), linear_srgb_to_oklab(c));
        println!(
            "{name}: srgb {srgb:.4?}\n  adaptive {a:.4?}\n  css      {c:.4?}\n  ΔE_OK {de:.4}"
        );
    }
}
