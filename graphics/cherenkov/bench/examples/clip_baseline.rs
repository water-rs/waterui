//! Measures what the pre-#96 channel-clip presentation would have scored
//! against the new gamut-mapped oracle: for each corpus scene handed in on
//! `argv`, renders the f64 reference, presents it through both the
//! historical clamp and the analytic map, and reports
//! `compare(mapped, clipped)` FLIP. The engine's presented output already
//! matches the mapped reference (~0 FLIP), so this number is the error the
//! clip behavior would have shown on the same scene.

use cherenkov_oracle::color::{linear_p3_to_linear_srgb, srgb_encode};
use cherenkov_oracle::present::{present_srgb, presented_srgb_to_working, quantize_unorm8};
use cherenkov_oracle::{F32Image, Image, Renderer, metrics};
use cherenkov_scene::Scene;

/// The pre-#96 `present_srgb`: P3 -> sRGB, clamp each channel, encode.
fn present_srgb_clip(image: &Image) -> Image {
    Image {
        width: image.width,
        height: image.height,
        pixels: image
            .pixels
            .iter()
            .map(|&[r, g, b, a]| {
                let straight = if a > 0.0 {
                    [r / a, g / a, b / a]
                } else {
                    [0.0; 3]
                };
                let clipped = linear_p3_to_linear_srgb(straight).map(|c| c.clamp(0.0, 1.0));
                [
                    srgb_encode(clipped[0]) * a,
                    srgb_encode(clipped[1]) * a,
                    srgb_encode(clipped[2]) * a,
                    a,
                ]
            })
            .collect(),
    }
}

fn working(presented: &Image) -> F32Image {
    let quantized = quantize_unorm8(presented);
    F32Image::from_f64(&Image {
        width: quantized.width,
        height: quantized.height,
        pixels: quantized
            .pixels
            .iter()
            .map(|&p| presented_srgb_to_working(p))
            .collect(),
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for dir in std::env::args().skip(1) {
        let dir = std::path::PathBuf::from(dir);
        let scene = Scene::load(&dir)?;
        let renderer = Renderer::new(scene.width as usize, scene.height as usize);
        let image = renderer.render_image(&scene, &dir)?;
        let reference = working(&present_srgb(1.0, &image));
        let clipped = working(&present_srgb_clip(&image));
        let (m, _) = metrics::compare(&reference, &clipped);
        println!(
            "{:24} clip-vs-map flip_mean={:.6} flip_max={:.4}",
            dir.file_name().unwrap_or_default().to_string_lossy(),
            m.flip_mean,
            m.flip_max
        );
    }
    Ok(())
}
