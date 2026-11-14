//! Deterministic compressed fixtures: four constant blocks with different
//! colours and alpha, including padding outside a logical 7×5 image.
use cherenkov_scene::kurbo::{Affine, Rect};
use cherenkov_scene::{
    Color, ColorSpace, Extend, ImageColorSpace, ImageCompression, ImageEncoding, ImagePaint, Paint,
    ResourceHash, Sampling, Shape,
};

fn constant_block(format: ImageCompression, color: [u8; 4]) -> [u8; 16] {
    let mut block = [0_u8; 16];
    match format {
        ImageCompression::Astc4x4 => {
            block[..8].copy_from_slice(&[0xfc, 0xfd, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
            for (bytes, channel) in block[8..].as_chunks_mut::<2>().0.iter_mut().zip(color) {
                *bytes = (u16::from(channel) * 257).to_le_bytes();
            }
        }
        ImageCompression::Bc7 => {
            // Mode 6: identical seven-bit endpoints, shared zero low bits,
            // and zero indices. Authored fixture channels are all even.
            assert!(color.iter().all(|c| c & 1 == 0));
            let mut cursor = 0;
            let mut put = |value: u8, count: usize| {
                for bit in 0..count {
                    block[cursor / 8] |= u8::from(value & (1 << bit) != 0) << (cursor % 8);
                    cursor += 1;
                }
            };
            put(0x40, 7);
            for channel in color {
                put(channel >> 1, 7);
                put(channel >> 1, 7);
            }
        }
        ImageCompression::Etc2Rgba => {
            // EAC alpha base with zero multiplier; RGB individual mode,
            // identical subblocks, table zero and selector zero (+2).
            block[0] = color[3];
            for (byte, channel) in block[8..11].iter_mut().zip(color) {
                let nibble = channel / 17;
                *byte = (nibble << 4) | nibble;
            }
        }
    }
    block
}

pub fn generate(corpus: &mut super::Corpus) {
    for (name, format) in [
        ("astc", ImageCompression::Astc4x4),
        ("bc7", ImageCompression::Bc7),
        ("etc2", ImageCompression::Etc2Rgba),
    ] {
        let blob: Vec<u8> = [
            [254, 0, 64, 0],
            [128, 32, 96, 128],
            [16, 222, 90, 254],
            [0, 128, 254, 64],
        ]
        .into_iter()
        .flat_map(|c| constant_block(format, c))
        .collect();
        for (space_name, color_space) in [
            ("srgb", ImageColorSpace::Srgb),
            ("p3", ImageColorSpace::DisplayP3),
            ("linear-srgb", ImageColorSpace::LinearSrgb),
            ("linear-p3", ImageColorSpace::LinearP3),
        ] {
            for premultiplied in [false, true] {
                let alpha = if premultiplied { "premul" } else { "straight" };
                let encoding = ImageEncoding::BlockCompressed {
                    format,
                    width: 7,
                    height: 5,
                    color_space,
                    premultiplied,
                };
                corpus.scene_with_blobs(
                    format!("img-{name}-{space_name}-{alpha}"),
                    128,
                    96,
                    Color::new(ColorSpace::LinearP3, [0.1, 0.1, 0.1, 1.0]),
                    |layer| {
                        let hash = ResourceHash::of(&blob);
                        layer.image_encoded(
                            hash,
                            encoding,
                            Rect::new(4.0, 4.0, 60.0, 44.0),
                            Sampling::Nearest,
                        );
                        layer.image_encoded(
                            hash,
                            encoding,
                            Rect::new(68.0, 4.0, 124.0, 44.0),
                            Sampling::Bilinear,
                        );
                        layer.fill(
                            Shape::rect(4.0, 52.0, 120.0, 40.0),
                            Paint::Image(ImagePaint {
                                image: hash,
                                encoding,
                                transform: Affine::translate((4.0, 52.0)) * Affine::scale(4.0),
                                extend_x: Extend::Repeat,
                                extend_y: Extend::Reflect,
                                sampling: Sampling::Bilinear,
                            }),
                        );
                    },
                    vec![blob.clone()],
                );
            }
        }
    }
}
