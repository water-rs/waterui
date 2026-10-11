//! GPU gamut-map sweep for #158: pushes the #96 sweep's sample domain
//! through the real `present.wgsl` sRGB shader-encode path
//! (`OutputColor::Srgb` into `Rgba8Unorm`) on the local GPU and reports the
//! shader output's `ΔE_OK` against both `f64` references — the CSS Color 4
//! spec map and dev's adaptive analytic map — plus hue shift against the
//! input. This is the on-GPU twin of `cherenkov-bench gamut-sweep`: it
//! scores whichever map the built `present.wgsl` carries.
//!
//! Run: `cargo run --locked --release -p cherenkov-bench --features
//! cherenkov --example gamut_gpu_sweep`

use cherenkov_gpu::GpuConfig;
use cherenkov_gpu::interop::{
    OutputAlpha, OutputColor, Presenter, SharedDevice, TextureOutput, shader_delivery, wgpu,
};
use cherenkov_oracle::color::srgb_encode;
use cherenkov_oracle::gamut::{delta_e_ok, gamut_map_srgb_analytic, linear_srgb_to_oklab};
use cherenkov_oracle::tone::tone_map;

/// `present.wgsl`'s simplified P3 → linear sRGB matrix (the same
/// constants, in `f64`).
const P3_TO_LINEAR_SRGB: [[f64; 3]; 3] = [
    [1.224_940_2, -0.224_940_2, 0.0],
    [-0.042_056_95, 1.042_056_9, 0.0],
    [-0.019_637_55, -0.078_636_05, 1.098_273_6],
];

fn mat3_mul(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0].mul_add(v[0], m[0][1].mul_add(v[1], m[0][2] * v[2])),
        m[1][0].mul_add(v[0], m[1][1].mul_add(v[1], m[1][2] * v[2])),
        m[2][0].mul_add(v[0], m[2][1].mul_add(v[1], m[2][2] * v[2])),
    ]
}

fn srgb_decode(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// The sweep's measurement reference: CSS Color 4 gamut mapping — `OKLCh`
/// chroma binary search with the local-MINDE rule (`JND` = 0.02 `ΔE_OK`,
/// epsilon = 1e-4). Mirrors `bench/src/gamut_sweep.rs`.
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
        let current = cherenkov_oracle::gamut::oklab_to_linear_srgb(cur_lab);
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

/// The #96 sweep's sample domain in the working space (linear P3), pre
/// conversion — what the shader's `p3_to_linear_srgb` then maps into the
/// out-of-gamut sRGB samples.
fn p3_samples() -> Vec<[f64; 3]> {
    let mut samples = Vec::new();
    let n = 65;
    for face in 0..6 {
        for i in 0..n {
            for j in 0..n {
                let u = f64::from(i) / f64::from(n - 1);
                let v = f64::from(j) / f64::from(n - 1);
                let fixed = f64::from(face % 2);
                samples.push(match face / 2 {
                    0 => [fixed, u, v],
                    1 => [u, fixed, v],
                    _ => [u, v, fixed],
                });
            }
        }
    }
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
                samples.push([
                    t.mul_add(corner[0] - g0, g0),
                    t.mul_add(corner[1] - g0, g0),
                    t.mul_add(corner[2] - g0, g0),
                ]);
            }
        }
    }
    samples
}

fn hue_deg(lab: [f64; 3]) -> f64 {
    lab[2].atan2(lab[1]).to_degrees().rem_euclid(360.0)
}

fn hue_diff_deg(a: [f64; 3], b: [f64; 3]) -> f64 {
    let d = (hue_deg(a) - hue_deg(b)).abs().rem_euclid(360.0);
    d.min(360.0 - d)
}

fn stats(values: &mut [f64]) -> (f64, f64, f64, f64) {
    values.sort_by(f64::total_cmp);
    let n = values.len();
    let n_f64 = f64::from(u32::try_from(n).expect("sweep samples fit u32"));
    (
        values.iter().sum::<f64>() / n_f64,
        values[n / 2],
        values[(n * 99 / 100).min(n - 1)],
        values[n - 1],
    )
}

#[expect(
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    reason = "linear setup-render-measure sequence; sizes fit u32"
)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let samples = p3_samples();
    // The shader applies tone_map(min(headroom, 1)) first — compute the
    // same effective sRGB input in f64 so both sides see identical data.
    let srgb_inputs: Vec<[f64; 3]> = samples
        .iter()
        .map(|&p3| mat3_mul(&P3_TO_LINEAR_SRGB, tone_map(1.0, p3)))
        .collect();
    let out_of_gamut = srgb_inputs
        .iter()
        .filter(|s| !s.iter().all(|&c| (0.0..=1.0).contains(&c)))
        .count();

    // Pack every sample into one wide Rgba16Float source (opaque alpha).
    let width = 1024u32;
    let height = (samples.len() as u32).div_ceil(width);
    let config = GpuConfig::default();
    let shared = SharedDevice::create(&config)?;
    let device = &shared.device;
    let queue = &shared.queue;
    let source = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sweep source"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut data = Vec::with_capacity((width * height * 8) as usize);
    for y in 0..height {
        for x in 0..width {
            let i = (y * width + x) as usize;
            let p3 = samples.get(i).copied().unwrap_or([0.5; 3]);
            for c in p3 {
                data.extend_from_slice(&half::f16::from_f64(c).to_le_bytes());
            }
            data.extend_from_slice(&half::f16::from_f32(1.0).to_le_bytes());
        }
    }
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &source,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 8),
            rows_per_image: Some(height),
        },
        source.size(),
    );
    let destination = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sweep output"),
        size: source.size(),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let delivery = shader_delivery(shared.adapter.get_info().backend, device)?;
    let mut presenter = Presenter::new(device, delivery);
    presenter.texture(
        device,
        queue,
        &source.create_view(&wgpu::TextureViewDescriptor::default()),
        TextureOutput {
            texture: &destination,
            color: OutputColor::Srgb,
            alpha: OutputAlpha::Opaque,
            headroom: 1.0,
        },
    );
    let row = (width * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("sweep readback"),
        size: u64::from(row * height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        destination.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(height),
            },
        },
        destination.size(),
    );
    let submission = queue.submit([encoder.finish()]);
    let (send, recv) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
    device.poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: Some(std::time::Duration::from_secs(60)),
    })?;
    recv.recv()??;
    let bytes = buffer.slice(..).get_mapped_range().unwrap();
    let gpu_linear = |i: usize| -> [f64; 3] {
        let base = (i as u32 / width) as usize * row as usize + (i as u32 % width) as usize * 4;
        [
            srgb_decode(f64::from(bytes[base]) / 255.0),
            srgb_decode(f64::from(bytes[base + 1]) / 255.0),
            srgb_decode(f64::from(bytes[base + 2]) / 255.0),
        ]
    };

    let mut de_css = Vec::new();
    let mut de_adaptive = Vec::new();
    let mut de_f64_adaptive_css = Vec::new();
    let mut hue = Vec::new();
    let mut hues_css = Vec::new();
    let mut hues_adaptive = Vec::new();
    let mut lsb_css = 0u32;
    let mut lsb_adaptive = 0u32;
    let mut in_gamut_exact = true;
    for (i, &srgb) in srgb_inputs.iter().enumerate() {
        let got = gpu_linear(i);
        if srgb.iter().all(|&c| (0.0..=1.0).contains(&c)) {
            // The map is the identity in gamut: quantization alone may
            // shift the byte, the encoded value may not.
            in_gamut_exact &=
                delta_e_ok(linear_srgb_to_oklab(got), linear_srgb_to_oklab(srgb)) < 0.01;
            continue;
        }
        let css = gamut_map_srgb_css(srgb);
        let adaptive = gamut_map_srgb_analytic(srgb);
        de_css.push(delta_e_ok(
            linear_srgb_to_oklab(got),
            linear_srgb_to_oklab(css),
        ));
        de_adaptive.push(delta_e_ok(
            linear_srgb_to_oklab(got),
            linear_srgb_to_oklab(adaptive),
        ));
        hue.push(hue_diff_deg(
            linear_srgb_to_oklab(got),
            linear_srgb_to_oklab(srgb),
        ));
        de_f64_adaptive_css.push(delta_e_ok(
            linear_srgb_to_oklab(adaptive),
            linear_srgb_to_oklab(css),
        ));
        hues_css.push(hue_diff_deg(
            linear_srgb_to_oklab(css),
            linear_srgb_to_oklab(srgb),
        ));
        hues_adaptive.push(hue_diff_deg(
            linear_srgb_to_oklab(adaptive),
            linear_srgb_to_oklab(srgb),
        ));
        for c in 0..3 {
            let byte = |v: f64| (srgb_encode(v.clamp(0.0, 1.0)) * 255.0).round() as i32;
            let base = (i as u32 / width) as usize * row as usize + (i as u32 % width) as usize * 4;
            let g = i32::from(bytes[base + c]);
            lsb_css = lsb_css.max(g.abs_diff(byte(css[c])));
            lsb_adaptive = lsb_adaptive.max(g.abs_diff(byte(adaptive[c])));
        }
    }
    drop(bytes);
    buffer.unmap();

    let (de_css_mean, de_css_p50, de_css_p99, de_css_max) = stats(&mut de_css);
    let (de_adp_mean, de_adp_p50, de_adp_p99, de_adp_max) = stats(&mut de_adaptive);
    let (_, hue_p50, hue_p99, hue_max) = stats(&mut hue);
    let (_, hue_css_p50, hue_css_p99, hue_css_max) = stats(&mut hues_css);
    let (_, hue_adp_p50, hue_adp_p99, hue_adp_max) = stats(&mut hues_adaptive);
    let (f64_mean, f64_p50, f64_p99, f64_max) = stats(&mut de_f64_adaptive_css);
    println!("gpu gamut sweep: {out_of_gamut} out-of-gamut samples through present.wgsl");
    println!(
        "  ΔE_OK vs CSS reference:      mean {de_css_mean:.5}  p50 {de_css_p50:.5}  p99 {de_css_p99:.5}  max {de_css_max:.5}"
    );
    println!(
        "  ΔE_OK vs adaptive reference: mean {de_adp_mean:.5}  p50 {de_adp_p50:.5}  p99 {de_adp_p99:.5}  max {de_adp_max:.5}"
    );
    println!("  hue° vs input:               p50 {hue_p50:.4}  p99 {hue_p99:.4}  max {hue_max:.4}");
    println!(
        "  hue° vs input, f64 CSS map:      p50 {hue_css_p50:.4}  p99 {hue_css_p99:.4}  max {hue_css_max:.4}"
    );
    println!(
        "  hue° vs input, f64 adaptive map: p50 {hue_adp_p50:.4}  p99 {hue_adp_p99:.4}  max {hue_adp_max:.4}"
    );
    println!(
        "  ΔE_OK f64 adaptive vs f64 CSS on this domain: mean {f64_mean:.5}  p50 {f64_p50:.5}  p99 {f64_p99:.5}  max {f64_max:.5}"
    );
    println!("  max |gpu - reference| in unorm8 LSB: css {lsb_css}  adaptive {lsb_adaptive}");
    println!("  in-gamut pixels within 0.01 ΔE of input: {in_gamut_exact}");
    Ok(())
}
