//! Registered images and image paint on lavapipe.

#![expect(
    clippy::cast_lossless,
    clippy::excessive_precision,
    clippy::suboptimal_flops,
    reason = "test tolerances and srgb8 conversions"
)]

use cherenkov::kurbo::{Affine, Rect};
use cherenkov::{__engine_fn as split_fn, __engine_test as split_test, __engine_wait as wait};
use cherenkov::{Draw, Extend, ImagePattern, Paint, Sampling};
use cherenkov::{
    Engine, EngineError, Image, ImageColorSpace, ImageData, Offscreen, OffscreenFormat, Rgba8,
};
use cherenkov_gpu::{Gpu, GpuConfig};

split_fn! {
/// An engine, or `None` when no adapter exists.
fn engine() -> Option<Engine<Gpu>> {
    match wait!(Engine::<Gpu>::new(GpuConfig::default())) {
        Ok(engine) => Some(engine),
        Err(EngineError::Backend(_)) => None,
        Err(e) => panic!("engine init failed: {e}"),
    }
}
}

/// sRGB→XYZ→P3 with the same constants the render thread uploads with.
const SRGB_TO_XYZ: [[f32; 3]; 3] = [
    [0.412_390_7, 0.357_584_33, 0.180_480_79],
    [0.212_639, 0.715_168_7, 0.072_192_32],
    [0.019_330_82, 0.119_194_76, 0.950_532_14],
];
const XYZ_TO_P3: [[f32; 3]; 3] = [
    [2.493_497, -0.931_383_6, -0.402_710_77],
    [-0.829_488_93, 1.762_664, 0.023_624_687],
    [0.035_845_827, -0.076_172_38, 0.956_884_5],
];

fn srgb_decode(c: u8) -> f32 {
    let u = c as f32 / 255.0;
    if u <= 0.040_45 {
        u / 12.92
    } else {
        ((u + 0.055) / 1.055).powf(2.4)
    }
}

fn mat_vec(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

split_fn! {
fn assert_image(surface: &cherenkov::Surface<Gpu>) -> Result<(), Box<dyn std::error::Error>> {
    let rb = wait!(surface.readback())?;
    let px = |x: u32, y: u32| rb.pixels[(y * rb.width + x) as usize];
    close_px(px(16, 16), premul_p3([255, 0, 0, 255]));
    close_px(px(48, 16), premul_p3([0, 255, 0, 255]));
    close_px(px(16, 48), premul_p3([0, 0, 255, 255]));
    close_px(px(48, 48), premul_p3([255, 255, 255, 128]));
    Ok(())
}
}

/// The premultiplied linear-P3 value of an sRGB8 texel, as uploaded.
fn premul_p3(c: [u8; 4]) -> [f32; 4] {
    let srgb = [srgb_decode(c[0]), srgb_decode(c[1]), srgb_decode(c[2])];
    let p3 = mat_vec(&XYZ_TO_P3, mat_vec(&SRGB_TO_XYZ, srgb));
    let a = c[3] as f32 / 255.0;
    [p3[0] * a, p3[1] * a, p3[2] * a, a]
}

/// Compare a readback texel (premultiplied linear P3 f32) to an expected
/// premultiplied-P3 colour.
fn close_px(px: [f32; 4], want_p3_premul: [f32; 4]) {
    for (g, w) in px.iter().zip(want_p3_premul) {
        assert!((g - w).abs() < 2e-2, "{px:?} vs {want_p3_premul:?}");
    }
}

/// A 2×2 image: opaque red / opaque green on row 0, opaque blue /
/// half-alpha white on row 1.
fn two_by_two(engine: &Engine<Gpu>) -> Image<Rgba8> {
    engine
        .image(
            ImageData::<Rgba8>::new(
                2,
                2,
                vec![
                    255, 0, 0, 255, // red
                    0, 255, 0, 255, // green
                    0, 0, 255, 255, // blue
                    255, 255, 255, 128, // half white
                ],
            )
            .unwrap()
            .color_space(ImageColorSpace::Srgb),
        )
        .unwrap()
}

split_test! {
fn an_image_draws_nearest() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let image = two_by_two(&engine);
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.image(image.id(), Rect::new(0., 0., 64., 64.), Sampling::Nearest);
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let rb = wait!(surface.readback())?;
    let px = |x: u32, y: u32| rb.pixels[(y * rb.width + x) as usize];
    close_px(px(16, 16), premul_p3([255, 0, 0, 255]));
    close_px(px(48, 16), premul_p3([0, 255, 0, 255]));
    close_px(px(16, 48), premul_p3([0, 0, 255, 255]));
    close_px(px(48, 48), premul_p3([255, 255, 255, 128]));
    Ok(())
}
}

split_test! {
fn an_image_interpolates_bilinear() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let image = two_by_two(&engine);
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.image(image.id(), Rect::new(0., 0., 64., 64.), Sampling::Linear);
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let rb = wait!(surface.readback())?;
    // Pixel centre (32,32) sits exactly between the four texels.
    let mut avg = [0f32; 4];
    for c in [
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [255, 255, 255, 128],
    ] {
        let p = premul_p3(c);
        for i in 0..4 {
            avg[i] += p[i] * 0.25;
        }
    }
    let px = rb.pixels[(32 * rb.width + 32) as usize];
    for (g, w) in px.iter().zip(avg) {
        assert!((g - w).abs() < 3e-2, "{px:?} vs {avg:?}");
    }
    Ok(())
}
}

split_test! {
fn an_image_pattern_repeats() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let image = two_by_two(&engine);
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.fill(
                Rect::new(0., 0., 64., 64.),
                Paint::Image(ImagePattern {
                    image: image.id(),
                    transform: Affine::IDENTITY,
                    extend_x: Extend::Repeat,
                    extend_y: Extend::Repeat,
                    sampling: Sampling::Nearest,
                }),
            );
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let rb = wait!(surface.readback())?;
    let px = |x: u32, y: u32| rb.pixels[(y * rb.width + x) as usize];
    // Identity transform maps image pixels 1:1; texel (0,0) is red and
    // wraps every 2 px — a pixel centre at x≈40.5 lands on texel 0.
    close_px(px(40, 0), premul_p3([255, 0, 0, 255]));
    close_px(px(41, 1), premul_p3([255, 255, 255, 128]));
    Ok(())
}
}

split_test! {
fn an_image_pattern_with_extend_none_is_transparent() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let image = two_by_two(&engine);
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.fill(
                Rect::new(0., 0., 64., 64.),
                Paint::Image(ImagePattern {
                    image: image.id(),
                    transform: Affine::IDENTITY,
                    extend_x: Extend::None,
                    extend_y: Extend::None,
                    sampling: Sampling::Nearest,
                }),
            );
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let rb = wait!(surface.readback())?;
    let px = |x: u32, y: u32| rb.pixels[(y * rb.width + x) as usize];
    close_px(px(0, 0), premul_p3([255, 0, 0, 255]));
    assert_eq!(px(8, 8), [0.0; 4], "outside the image must be clear");
    Ok(())
}
}

/// f16 texel bytes for one pixel.
fn f16_px(px: [f32; 4]) -> [u8; 8] {
    let mut out = [0u8; 8];
    for (i, v) in px.iter().enumerate() {
        out[2 * i..2 * i + 2].copy_from_slice(&half::f16::from_f32(*v).to_le_bytes());
    }
    out
}

/// The premultiplied linear-P3 value of a linear-sRGB f16 texel, as uploaded.
fn linear_srgb_to_p3_premul(c: [f32; 4]) -> [f32; 4] {
    let p3 = mat_vec(&XYZ_TO_P3, mat_vec(&SRGB_TO_XYZ, [c[0], c[1], c[2]]));
    [p3[0] * c[3], p3[1] * c[3], p3[2] * c[3], c[3]]
}

split_test! {
/// An `Rgba16F` upload survives with HDR and P3-only channels intact:
/// 8x red, a half-alpha teal, a P3-only red and the premultiplied form.
fn an_f16_image_keeps_hdr_and_wide_gamut() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    // An 8x-red in linear sRGB (HDR through the primaries' matrix) and a
    // straight-alpha teal above SDR white.
    let mut data = Vec::new();
    data.extend_from_slice(&f16_px([8.0, 0.0, 0.0, 1.0]));
    data.extend_from_slice(&f16_px([0.0, 2.0, 0.25, 0.5]));
    let hdr = engine.image(
        ImageData::<cherenkov::Rgba16F>::new(2, 1, data)
            .unwrap()
            .color_space(ImageColorSpace::LinearSrgb),
    )?;
    // Pure P3 red — a colour the sRGB gamut cannot express.
    let p3 = engine.image(
        ImageData::<cherenkov::Rgba16F>::new(1, 1, f16_px([1.0, 0.0, 0.0, 1.0]))
            .unwrap()
            .color_space(ImageColorSpace::LinearP3),
    )?;
    // Premultiplied [4,0,0,0.5] is the same texel as straight [8,0,0,0.5].
    let premul = engine.image(
        ImageData::<cherenkov::Rgba16F>::new(1, 1, f16_px([4.0, 0.0, 0.0, 0.5]))
            .unwrap()
            .color_space(ImageColorSpace::LinearSrgb)
            .premultiplied(),
    )?;
    let surface = wait!(engine.surface(Offscreen::new((8, 2), OffscreenFormat::LinearF16)))?;
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.image(hdr.id(), Rect::new(0., 0., 4., 2.), Sampling::Nearest);
            c.image(p3.id(), Rect::new(4., 0., 6., 2.), Sampling::Nearest);
            c.image(premul.id(), Rect::new(6., 0., 8., 2.), Sampling::Nearest);
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let rb = wait!(surface.readback())?;
    let px = |x: u32, y: u32| rb.pixels[(y * rb.width + x) as usize];
    close_px(px(0, 0), linear_srgb_to_p3_premul([8.0, 0.0, 0.0, 1.0]));
    close_px(px(2, 0), linear_srgb_to_p3_premul([0.0, 2.0, 0.25, 0.5]));
    close_px(px(4, 0), [1.0, 0.0, 0.0, 1.0]);
    close_px(px(6, 0), linear_srgb_to_p3_premul([8.0, 0.0, 0.0, 0.5]));
    Ok(())
}
}

split_test! {
/// An image larger than the device's texture limit is a rejection only the
/// backend can detect: registration returns the handle, and the render
/// that draws the image fails naming it and the backend's reason.
fn an_image_beyond_the_texture_limit_fails_the_render_that_draws_it()
-> Result<(), Box<dyn std::error::Error>> {
    let shared = match wait!(cherenkov_gpu::interop::SharedDevice::create(&GpuConfig::default())) {
        Ok(shared) => shared,
        Err(EngineError::Backend(_)) => return Ok(()),
        Err(e) => panic!("device creation failed: {e}"),
    };
    let width = shared.device.limits().max_texture_dimension_2d + 1;
    let engine = wait!(Engine::<Gpu>::new(GpuConfig {
        device: Some(shared),
        ..GpuConfig::default()
    }))?;
    let surface = wait!(engine.surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16)))?;
    let image = engine.image(ImageData::<Rgba8>::new(
        width,
        1,
        vec![255; width as usize * 4],
    )?)?;
    surface.update(|tx| {
        tx[surface.root()].record(|c| {
            c.image(image.id(), Rect::new(0.0, 0.0, 8.0, 8.0), Sampling::Nearest);
        });
    });
    match wait!(engine.render(cherenkov::FrameTime::now())) {
        Err(cherenkov::RenderError::Rejected { resource, reason }) => {
            assert_eq!(resource, cherenkov::ResourceId::Image(image.id()));
            assert!(
                matches!(*reason, cherenkov::ResourceError::Image(_)),
                "{reason}"
            );
        }
        other => panic!("an oversized image was drawn: {other:?}"),
    }
    Ok(())
}
}

split_test! {
/// An image whose last handle drops between recording the content that
/// replaces it and installing that content still draws correctly in a
/// render issued before the install; the render after the install draws
/// the new content and frees the image (#199).
fn an_image_released_before_its_replacement_is_installed_still_draws()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let image = two_by_two(&engine);
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    let pictured = surface.layer();
    let marker = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&pictured);
        tx[surface.root()].push(&marker);
        tx[&pictured].record(|c| {
            c.image(image.id(), Rect::new(0., 0., 64., 64.), Sampling::Nearest);
        });
        tx[&marker].record(|c| {
            c.fill(
                Rect::new(60., 60., 64., 64.),
                cherenkov::WorkingColor::BLACK,
            );
        });
    });

    wait!(engine.render(cherenkov::FrameTime::now()))?;
    wait!(assert_image(&surface))?;

    let replacement = surface.record(|c| {
        c.fill(Rect::new(0., 0., 64., 64.), cherenkov::WorkingColor::WHITE);
    });
    drop(image);
    // A property change elsewhere redraws the surface while the installed
    // content still draws the released image.
    surface.update(|tx| {
        tx[&marker].opacity(0.5f32);
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    wait!(assert_image(&surface))?;

    surface.update(|tx| {
        tx[&pictured].content(replacement);
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let rb = wait!(surface.readback())?;
    close_px(
        rb.pixels[(16 * rb.width + 16) as usize],
        [1.0, 1.0, 1.0, 1.0],
    );
    Ok(())
}
}
