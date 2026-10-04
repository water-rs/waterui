//! Shared-front-end image uploads, retained destinations and alpha metadata.

#![cfg(not(target_arch = "wasm32"))]
#![expect(
    clippy::excessive_precision,
    clippy::suboptimal_flops,
    reason = "test tolerances and srgb8 conversions"
)]

use cherenkov::kurbo::{Affine, Rect};
use cherenkov::{
    Draw, Engine, FrameTime, ImageColorSpace, ImageData, Offscreen, OffscreenFormat, Picture,
    Rgba8, Sampling, WorkingColor,
};
use cherenkov_cpu::{Raster, RasterConfig};
use nami::Binding;

#[test]
fn encoded_premultiplied_upload_matches_straight_alpha() {
    let engine = Engine::<Raster>::new(RasterConfig::default()).expect("engine");
    let straight = engine
        .image(
            ImageData::<Rgba8>::new(1, 1, vec![255, 0, 0, 128])
                .expect("data")
                .color_space(ImageColorSpace::DisplayP3),
        )
        .expect("straight");
    let premul = engine
        .image(
            ImageData::<Rgba8>::new(1, 1, vec![128, 0, 0, 128])
                .expect("data")
                .color_space(ImageColorSpace::DisplayP3)
                .premultiplied(),
        )
        .expect("premul");
    let surface = engine
        .surface(Offscreen::new((4, 2), OffscreenFormat::LinearF32))
        .expect("surface");
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.image(
                straight.id(),
                Rect::new(0.0, 0.0, 2.0, 2.0),
                Sampling::Nearest,
            );
            c.image(
                premul.id(),
                Rect::new(2.0, 0.0, 4.0, 2.0),
                Sampling::Nearest,
            );
        }));
    });
    engine.render(FrameTime::now()).expect("render");
    let pixels = surface.readback().expect("pixels").pixels;
    assert_eq!(pixels[0].map(f32::to_bits), pixels[2].map(f32::to_bits));
    let alpha = 128.0_f32 / 255.0;
    assert!((pixels[0][0] - alpha).abs() < 1e-6 && (pixels[0][3] - alpha).abs() < 1e-6);
}

#[test]
fn image_destination_is_live_and_static_picture_stays_retained() {
    let engine = Engine::<Raster>::new(RasterConfig::default()).expect("engine");
    let image = engine
        .image(ImageData::<Rgba8>::new(1, 1, vec![255, 255, 255, 255]).expect("data"))
        .expect("image");
    let surface = engine
        .surface(Offscreen::new((24, 24), OffscreenFormat::LinearF32))
        .expect("surface");
    let dst = Binding::container(Rect::new(4.0, 4.0, 8.0, 8.0));
    let fixed = Picture::record(|c| c.fill(Rect::new(0.0, 0.0, 2.0, 2.0), WorkingColor::WHITE));
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.picture(&fixed, Affine::IDENTITY);
            c.image(image.id(), dst.clone(), Sampling::Linear);
        }));
    });
    engine.render(FrameTime::now()).expect("initial");
    assert_eq!(engine.stats().commands_lowered, 2);
    dst.set(Rect::new(12.0, 12.0, 16.0, 16.0));
    engine.render(FrameTime::now()).expect("edit");
    assert_eq!(engine.stats().commands_lowered, 1);
    let pixels = surface.readback().expect("pixels").pixels;
    assert!(pixels[13 * 24 + 13][3] > 0.999);
    assert_eq!(pixels[5 * 24 + 5][3].to_bits(), 0.0_f32.to_bits());
    engine.render(FrameTime::now()).expect("idle");
    assert_eq!(engine.stats().commands_lowered, 0);
}

/// f16 texel bytes for one pixel.
fn f16_px(px: [f32; 4]) -> [u8; 8] {
    let mut out = [0u8; 8];
    for (i, v) in px.iter().enumerate() {
        out[2 * i..2 * i + 2].copy_from_slice(&half::f16::from_f32(*v).to_le_bytes());
    }
    out
}

/// Linear sRGB -> linear Display P3, same matrices as the decode path.
fn srgb_to_p3(c: [f32; 3]) -> [f32; 3] {
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
    let xyz = [
        SRGB_TO_XYZ[0][0] * c[0] + SRGB_TO_XYZ[0][1] * c[1] + SRGB_TO_XYZ[0][2] * c[2],
        SRGB_TO_XYZ[1][0] * c[0] + SRGB_TO_XYZ[1][1] * c[1] + SRGB_TO_XYZ[1][2] * c[2],
        SRGB_TO_XYZ[2][0] * c[0] + SRGB_TO_XYZ[2][1] * c[1] + SRGB_TO_XYZ[2][2] * c[2],
    ];
    [
        XYZ_TO_P3[0][0] * xyz[0] + XYZ_TO_P3[0][1] * xyz[1] + XYZ_TO_P3[0][2] * xyz[2],
        XYZ_TO_P3[1][0] * xyz[0] + XYZ_TO_P3[1][1] * xyz[1] + XYZ_TO_P3[1][2] * xyz[2],
        XYZ_TO_P3[2][0] * xyz[0] + XYZ_TO_P3[2][1] * xyz[1] + XYZ_TO_P3[2][2] * xyz[2],
    ]
}

/// An `Rgba16F` upload survives with HDR and P3-only channels intact:
/// 8x red, a half-alpha teal, a P3-only red and the premultiplied form.
#[test]
fn rgba16f_upload_keeps_hdr_and_wide_gamut() {
    let engine = Engine::<Raster>::new(RasterConfig::default()).expect("engine");
    // An 8x-red in linear sRGB (HDR through the primaries' matrix) and a
    // straight-alpha teal with a negative channel.
    let mut data = Vec::new();
    data.extend_from_slice(&f16_px([8.0, 0.0, 0.0, 1.0]));
    data.extend_from_slice(&f16_px([0.0, 2.0, 0.25, 0.5]));
    let hdr = engine
        .image(
            ImageData::<cherenkov::Rgba16F>::new(2, 1, data)
                .expect("data")
                .color_space(ImageColorSpace::LinearSrgb),
        )
        .expect("image");
    // Pure P3 red — a colour the sRGB gamut cannot express.
    let p3 = engine
        .image(
            ImageData::<cherenkov::Rgba16F>::new(1, 1, f16_px([1.0, 0.0, 0.0, 1.0]))
                .expect("data")
                .color_space(ImageColorSpace::LinearP3),
        )
        .expect("image");
    // Premultiplied [4,0,0,0.5] is the same texel as straight [8,0,0,0.5].
    let premul = engine
        .image(
            ImageData::<cherenkov::Rgba16F>::new(1, 1, f16_px([4.0, 0.0, 0.0, 0.5]))
                .expect("data")
                .color_space(ImageColorSpace::LinearSrgb)
                .premultiplied(),
        )
        .expect("image");
    let surface = engine
        .surface(Offscreen::new((8, 2), OffscreenFormat::LinearF32))
        .expect("surface");
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.image(hdr.id(), Rect::new(0.0, 0.0, 4.0, 2.0), Sampling::Nearest);
            c.image(p3.id(), Rect::new(4.0, 0.0, 6.0, 2.0), Sampling::Nearest);
            c.image(
                premul.id(),
                Rect::new(6.0, 0.0, 8.0, 2.0),
                Sampling::Nearest,
            );
        }));
    });
    engine.render(FrameTime::now()).expect("render");
    let pixels = surface.readback().expect("pixels").pixels;
    // Texel 0 covers x in [0,2), texel 1 x in [2,4), the P3 image [4,6).
    let [r, g, b] = srgb_to_p3([8.0, 0.0, 0.0]);
    let got = pixels[0];
    for (g_, w) in got.iter().zip([r, g, b, 1.0]) {
        assert!((g_ - w).abs() < 1e-2, "{got:?} vs [{r}, {g}, {b}, 1.0]");
    }
    // Half-alpha texel: premultiplied P3 at half the straight colour.
    let [r, g, b] = srgb_to_p3([0.0, 2.0, 0.25]);
    let got = pixels[2];
    for (g_, w) in got.iter().zip([0.5 * r, 0.5 * g, 0.5 * b, 0.5]) {
        assert!((g_ - w).abs() < 1e-2, "{got:?} vs half-alpha teal");
    }
    let got = pixels[4];
    for (g_, w) in got.iter().zip([1.0, 0.0, 0.0, 1.0]) {
        assert!((g_ - w).abs() < 1e-2, "{got:?} vs P3 red");
    }
    let [r, g, b] = srgb_to_p3([8.0, 0.0, 0.0]);
    let got = pixels[6];
    for (g_, w) in got.iter().zip([0.5 * r, 0.5 * g, 0.5 * b, 0.5]) {
        assert!((g_ - w).abs() < 1e-2, "{got:?} vs premultiplied HDR red");
    }
}
