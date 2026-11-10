//! `OffscreenRenderer::render_drawing`: a `Drawing` becomes an image that
//! fills the requested pixel size, the same on the GPU and CPU renderers.
#![cfg(all(feature = "gpu", not(target_arch = "wasm32")))]

use std::path::PathBuf;

use cherenkov_gpu::Gpu;
use waterui_core::layout::{Point, Rect, Size};
use waterui_graphics::canvas::{Circle, Drawing, LinearGradient, StrokeStyle};
use waterui_graphics::{OffscreenRenderer, OffscreenSize, WorkingColor};

/// A 40x20 point drawing: a gradient band under a red block and a white ring.
fn drawing() -> Drawing {
    Drawing::record(Size::new(40.0, 20.0), |cx| {
        cx.fill(
            Rect::from_size(Size::new(40.0, 20.0)),
            LinearGradient::new(Point::new(0.0, 0.0), Point::new(40.0, 0.0))
                .stop(0.0, WorkingColor::new([0.0, 0.2, 1.0, 1.0]))
                .stop(1.0, WorkingColor::new([1.0, 0.8, 0.0, 1.0])),
        );
        cx.fill(
            Rect::new(Point::new(4.0, 4.0), Size::new(12.0, 12.0)),
            WorkingColor::new([0.9, 0.0, 0.0, 1.0]),
        );
        cx.stroke(
            Circle::new(Point::new(28.0, 10.0), 6.0),
            &StrokeStyle::new(2.0),
            WorkingColor::WHITE,
        );
    })
}

/// Where the rendered images are written for review.
fn out_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("render_drawing");
    std::fs::create_dir_all(&dir).expect("output directory");
    dir
}

#[test]
fn the_gpu_renderer_fills_the_requested_size_with_the_drawing() {
    let size = OffscreenSize::try_from_pixels(160, 80).expect("non-zero");
    let gpu = OffscreenRenderer::<Gpu>::new().expect("a GPU adapter is required on test hardware");
    let image = gpu.render_drawing(&drawing(), size).expect("gpu render");
    assert_eq!((image.width(), image.height()), (160, 80));
    image.save_png(out_dir().join("gpu.png")).expect("gpu png");

    // A different aspect than the drawing's own: it is scaled to fit.
    let square = OffscreenSize::try_from_pixels(80, 80).expect("non-zero");
    let image = gpu.render_drawing(&drawing(), square).expect("gpu render");
    assert_eq!((image.width(), image.height()), (80, 80));
    image
        .save_png(out_dir().join("gpu-square.png"))
        .expect("gpu square png");
}

#[cfg(feature = "cpu")]
mod cpu {
    use cherenkov_cpu::Raster;
    use cherenkov_oracle::F32Image;
    use cherenkov_oracle::metrics::assert_images_match;
    use waterui_graphics::OffscreenImage;

    use super::*;

    /// The image as premultiplied linear floats, the metric's interchange
    /// form.
    fn linear(image: &OffscreenImage) -> F32Image {
        let decode = |byte: u8| {
            let c = f32::from(byte) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let mut out = F32Image::new(image.width(), image.height());
        for y in 0..image.height() {
            for x in 0..image.width() {
                let [r, g, b, a] = image.pixel(x, y);
                let alpha = f32::from(a) / 255.0;
                out.pixels[(y * image.width() + x) as usize] = [
                    decode(r) * alpha,
                    decode(g) * alpha,
                    decode(b) * alpha,
                    alpha,
                ];
            }
        }
        out
    }

    #[test]
    fn a_drawing_renders_alike_on_the_gpu_and_cpu_renderers() {
        let drawing = drawing();
        let size = OffscreenSize::try_from_pixels(160, 80).expect("non-zero");
        let gpu =
            OffscreenRenderer::<Gpu>::new().expect("a GPU adapter is required on test hardware");
        let cpu = OffscreenRenderer::<Raster>::cpu().expect("CPU engine");

        let rasterized = cpu.render_drawing(&drawing, size).expect("cpu render");
        let accelerated = gpu.render_drawing(&drawing, size).expect("gpu render");

        assert_eq!((rasterized.width(), rasterized.height()), (160, 80));
        rasterized
            .save_png(out_dir().join("cpu.png"))
            .expect("cpu png");
        assert_images_match(&linear(&rasterized), &linear(&accelerated));
    }
}
