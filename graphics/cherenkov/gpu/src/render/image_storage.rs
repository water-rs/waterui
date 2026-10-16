//! Source storage and sampling metadata; compositing remains linear Display P3.
use cherenkov::{ImageColorSpace, ImageFormat, ImageUpload, ResourceError};
use std::borrow::Cow;

pub(super) const COMPRESSION_FEATURES: wgpu::Features = wgpu::Features::TEXTURE_COMPRESSION_ASTC
    .union(wgpu::Features::TEXTURE_COMPRESSION_BC)
    .union(wgpu::Features::TEXTURE_COMPRESSION_ETC2);

/// Image paint bits 12–16, `shader.wgsl`'s `image_working_texel`. Zero is
/// a texture already holding premultiplied linear Display P3: internal
/// textures, and `Rgba16F` sources converted at upload.
const SOURCE: u32 = 1 << 12;
/// The texels carry premultiplied alpha.
const PREMULTIPLIED: u32 = 1 << 13;
/// The sRGB transfer decode, evaluated per tap.
const TRANSFER: u32 = 1 << 14;
/// sRGB primaries, converted to Display P3 per tap.
const SRGB_PRIMARIES: u32 = 1 << 15;
/// The sRGB transfer decode, looked up in [`transfer_table`] by the
/// texel's 8-bit code.
const TRANSFER_TABLE: u32 = 1 << 16;

/// The sRGB transfer decode of every 8-bit code, as the shader's
/// `image_decode` binding holds it.
///
/// Each entry is the f64 decode the f16 upload conversion applies to a
/// straight-alpha code, rounded once to f32, so a straight 8-bit source
/// decodes to the same linear value whether it is converted at upload or
/// per tap.
pub(super) fn transfer_table() -> [f32; 256] {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the f64 decode is rounded to the shader's f32 once, by design"
    )]
    std::array::from_fn(|code| {
        super::srgb_decode_u8_f64(f64::from(u8::try_from(code).expect("256 codes")) / 255.0) as f32
    })
}

pub(super) struct Upload<'a> {
    pub bytes: Cow<'a, [u8]>,
    pub format: wgpu::TextureFormat,
    pub flags: u32,
}

impl<'a> Upload<'a> {
    pub fn new(image: &'a ImageUpload, features: wgpu::Features) -> Result<Self, ResourceError> {
        use wgpu::TextureFormat as T;
        // An f16 source is no smaller than the working texels, so it takes
        // its colour interpretation once, here, into the same storage.
        if image.format == ImageFormat::Rgba16F {
            return Ok(Self {
                bytes: Cow::Owned(working_f16(image)),
                format: T::Rgba16Float,
                flags: 0,
            });
        }
        // sRGB texture decoding can quantize dark values more coarsely than
        // the working f16 representation. Preserve the encoded bytes and
        // decode each tap in the shader, for both alpha interpretations.
        let rgba8 = T::Rgba8Unorm;
        let (native, required) = match image.format {
            ImageFormat::Rgba8 => (rgba8, wgpu::Features::empty()),
            ImageFormat::Astc4x4 => (
                T::Astc {
                    block: wgpu::AstcBlock::B4x4,
                    channel: wgpu::AstcChannel::Unorm,
                },
                wgpu::Features::TEXTURE_COMPRESSION_ASTC,
            ),
            ImageFormat::Bc7 => (T::Bc7RgbaUnorm, wgpu::Features::TEXTURE_COMPRESSION_BC),
            ImageFormat::Etc2Rgba => (T::Etc2Rgba8Unorm, wgpu::Features::TEXTURE_COMPRESSION_ETC2),
            ImageFormat::Rgba16F => unreachable!("converted at upload above"),
        };
        let (format, bytes) = if features.contains(required) {
            (native, Cow::Borrowed(image.data.as_ref()))
        } else {
            tracing::debug!(format = ?image.format, required = ?required, "decoding image because the device lacks its compression feature");
            (rgba8, Cow::Owned(decode(image)?))
        };
        let nonlinear = matches!(
            image.color_space,
            ImageColorSpace::Srgb | ImageColorSpace::DisplayP3
        );
        // A table entry is the decode of an 8-bit code. Straight alpha
        // decodes the stored code itself; premultiplied texels decode
        // their quotient by alpha, which is no code, and ASTC's UNORM
        // decode is 16-bit, so both evaluate the transfer function.
        let codes = format
            != T::Astc {
                block: wgpu::AstcBlock::B4x4,
                channel: wgpu::AstcChannel::Unorm,
            };
        let transfer = match (nonlinear, codes && !image.premultiplied) {
            (false, _) => 0,
            (true, true) => TRANSFER_TABLE,
            (true, false) => TRANSFER,
        };
        let flags = SOURCE
            | if image.premultiplied {
                PREMULTIPLIED
            } else {
                0
            }
            | transfer
            | if matches!(
                image.color_space,
                ImageColorSpace::Srgb | ImageColorSpace::LinearSrgb
            ) {
                SRGB_PRIMARIES
            } else {
                0
            };
        Ok(Self {
            bytes,
            format,
            flags,
        })
    }
}

/// `Rgba16F` upload bytes -> premultiplied linear-P3 f16 texels, the
/// working texel format.
fn working_f16(image: &ImageUpload) -> Vec<u8> {
    let mut data = Vec::with_capacity(image.data.len());
    for px in image.data.as_chunks::<8>().0 {
        let encoded = std::array::from_fn(|i| {
            f64::from(half::f16::from_le_bytes([px[2 * i], px[2 * i + 1]]))
        });
        // The texels keep extended (HDR and wide-gamut) values, so the
        // un-premultiplied quotient is unbounded.
        push_working_f16(
            encoded,
            image.color_space,
            image.premultiplied,
            f64::INFINITY,
            &mut data,
        );
    }
    data
}

/// Appends one encoded texel as a premultiplied linear-P3 f16 texel, the
/// working texel format.
///
/// Straight-alpha input decodes each channel; premultiplied input is
/// un-premultiplied in the encoded domain first, bounded at
/// `unpremultiplied_max`.
pub(super) fn push_working_f16(
    encoded: [f64; 4],
    color_space: ImageColorSpace,
    premultiplied: bool,
    unpremultiplied_max: f64,
    data: &mut Vec<u8>,
) {
    let alpha = encoded[3];
    // `min` also maps a NaN quotient to the bound.
    let decode = |v: f64| {
        if premultiplied && alpha > 0.0 {
            (v / alpha).min(unpremultiplied_max)
        } else {
            v
        }
    };
    let lin = match color_space {
        ImageColorSpace::LinearSrgb | ImageColorSpace::LinearP3 => {
            [decode(encoded[0]), decode(encoded[1]), decode(encoded[2])]
        }
        _ => [
            super::srgb_decode_u8_f64(decode(encoded[0])),
            super::srgb_decode_u8_f64(decode(encoded[1])),
            super::srgb_decode_u8_f64(decode(encoded[2])),
        ],
    };
    // sRGB-primaries input additionally needs the primaries' matrix;
    // Display P3 uses sRGB's transfer function, so the decode above
    // covers both encoded spaces. `LinearP3` is already the working
    // space: no transfer, no matrix.
    let lin_p3 = match color_space {
        ImageColorSpace::Srgb | ImageColorSpace::LinearSrgb => {
            let (to_xyz, to_p3) = (super::SRGB_TO_XYZ, super::XYZ_TO_P3);
            let [x, y, z] = [
                to_xyz[0][2].mul_add(lin[2], to_xyz[0][1].mul_add(lin[1], to_xyz[0][0] * lin[0])),
                to_xyz[1][2].mul_add(lin[2], to_xyz[1][1].mul_add(lin[1], to_xyz[1][0] * lin[0])),
                to_xyz[2][2].mul_add(lin[2], to_xyz[2][1].mul_add(lin[1], to_xyz[2][0] * lin[0])),
            ];
            [
                to_p3[0][2].mul_add(z, to_p3[0][1].mul_add(y, to_p3[0][0] * x)),
                to_p3[1][2].mul_add(z, to_p3[1][1].mul_add(y, to_p3[1][0] * x)),
                to_p3[2][2].mul_add(z, to_p3[2][1].mul_add(y, to_p3[2][0] * x)),
            ]
        }
        ImageColorSpace::DisplayP3 | ImageColorSpace::LinearP3 => lin,
    };
    for v in [
        alpha * lin_p3[0],
        alpha * lin_p3[1],
        alpha * lin_p3[2],
        alpha,
    ] {
        data.extend_from_slice(&half::f16::from_f64(v).to_le_bytes());
    }
}

pub(super) fn bytes(texture: &wgpu::Texture) -> u64 {
    storage_bytes(texture.format(), texture.width(), texture.height())
}

fn storage_bytes(format: wgpu::TextureFormat, width: u32, height: u32) -> u64 {
    let (bw, bh) = format.block_dimensions();
    u64::from(width.div_ceil(bw))
        * u64::from(height.div_ceil(bh))
        * u64::from(format.block_copy_size(None).expect("colour format"))
}

fn decode(image: &ImageUpload) -> Result<Vec<u8>, ResourceError> {
    let width = image.width as usize;
    let height = image.height as usize;
    let mut pixels = vec![0_u32; width * height];
    let result = match image.format {
        ImageFormat::Astc4x4 => {
            texture2ddecoder::decode_astc(&image.data, width, height, 4, 4, &mut pixels)
        }
        ImageFormat::Bc7 => texture2ddecoder::decode_bc7(&image.data, width, height, &mut pixels),
        ImageFormat::Etc2Rgba => {
            texture2ddecoder::decode_etc2_rgba8(&image.data, width, height, &mut pixels)
        }
        ImageFormat::Rgba8 | ImageFormat::Rgba16F => {
            unreachable!("uncompressed formats require no optional feature")
        }
    };
    result.map_err(|error| ResourceError::Image(format!("{:?} decode: {error}", image.format)))?;
    // The decoder's packed colour is 0xAARRGGBB, independent of host endian.
    Ok(pixels
        .into_iter()
        .flat_map(|p| {
            let [b, g, r, a] = p.to_le_bytes();
            [r, g, b, a]
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(format: ImageFormat, bytes: Vec<u8>) -> ImageUpload {
        ImageUpload {
            width: 3,
            height: 2,
            data: bytes.into(),
            color_space: ImageColorSpace::Srgb,
            premultiplied: false,
            format,
        }
    }

    #[test]
    fn eight_bit_sources_borrow_bytes_and_select_their_per_tap_decode() {
        for color_space in [
            ImageColorSpace::Srgb,
            ImageColorSpace::DisplayP3,
            ImageColorSpace::LinearSrgb,
            ImageColorSpace::LinearP3,
        ] {
            for premultiplied in [false, true] {
                let mut source = image(ImageFormat::Rgba8, vec![0; 24]);
                source.color_space = color_space;
                source.premultiplied = premultiplied;
                let upload = Upload::new(&source, wgpu::Features::empty()).unwrap();
                assert!(matches!(upload.bytes, Cow::Borrowed(_)));
                assert_eq!(upload.bytes.as_ptr(), source.data.as_ptr());
                assert_eq!(upload.format, wgpu::TextureFormat::Rgba8Unorm);
                assert_eq!(storage_bytes(upload.format, 3, 2), 24);
                assert_ne!(upload.flags & SOURCE, 0);
                assert_eq!((upload.flags & PREMULTIPLIED) != 0, premultiplied);
                assert_eq!(
                    (upload.flags & SRGB_PRIMARIES) != 0,
                    matches!(
                        color_space,
                        ImageColorSpace::Srgb | ImageColorSpace::LinearSrgb
                    )
                );
                let nonlinear = matches!(
                    color_space,
                    ImageColorSpace::Srgb | ImageColorSpace::DisplayP3
                );
                assert_eq!(
                    (upload.flags & TRANSFER_TABLE) != 0,
                    nonlinear && !premultiplied
                );
                assert_eq!((upload.flags & TRANSFER) != 0, nonlinear && premultiplied);
            }
        }
    }

    #[test]
    fn f16_sources_take_their_interpretation_at_upload() {
        let texel = |v: [f32; 4]| -> Vec<u8> {
            v.into_iter()
                .flat_map(|c| half::f16::from_f32(c).to_le_bytes())
                .collect()
        };
        let channels = |bytes: &[u8]| -> Vec<f32> {
            bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| half::f16::from_le_bytes(*c).to_f32())
                .collect()
        };
        let mut source = image(ImageFormat::Rgba16F, texel([2.0, 0.5, 0.0, 0.5]).repeat(6));
        source.color_space = ImageColorSpace::LinearP3;
        source.premultiplied = true;
        let upload = Upload::new(&source, wgpu::Features::empty()).unwrap();
        assert_eq!(upload.format, wgpu::TextureFormat::Rgba16Float);
        assert_eq!(upload.flags, 0);
        // Premultiplied linear P3 is already the working representation,
        // extended channels included.
        assert_eq!(channels(&upload.bytes), [2.0, 0.5, 0.0, 0.5].repeat(6));
        source.premultiplied = false;
        let upload = Upload::new(&source, wgpu::Features::empty()).unwrap();
        assert_eq!(channels(&upload.bytes), [1.0, 0.25, 0.0, 0.5].repeat(6));
    }

    #[test]
    fn transfer_table_reproduces_the_upload_conversion_of_opaque_codes() {
        let table = transfer_table();
        for code in 0..=255_u8 {
            let mut texel = Vec::new();
            push_working_f16(
                [f64::from(code) / 255.0, 0.0, 0.0, 1.0],
                ImageColorSpace::DisplayP3,
                false,
                1.0,
                &mut texel,
            );
            assert_eq!(
                half::f16::from_f32(table[usize::from(code)]),
                half::f16::from_le_bytes([texel[0], texel[1]]),
                "code {code}"
            );
        }
    }

    // ASTC LDR void-extent blocks carry a constant UNORM16 colour.
    fn astc() -> Vec<u8> {
        let mut block = vec![0xfc, 0xfd, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];
        for c in [201_u16, 81, 21, 129] {
            block.extend_from_slice(&(c * 257).to_le_bytes());
        }
        block
    }

    // BC7 mode 6, identical RGBA endpoints and zero interpolation indices.
    fn bc7() -> Vec<u8> {
        let mut block = vec![0_u8; 16];
        let mut cursor = 0;
        let mut put = |value: u32, bits: usize| {
            for bit in 0..bits {
                block[cursor / 8] |= u8::from(value & (1 << bit) != 0) << (cursor % 8);
                cursor += 1;
            }
        };
        put(64, 7);
        for channel in [100, 40, 10, 64] {
            put(channel, 7);
            put(channel, 7);
        }
        put(1, 1);
        put(1, 1);
        block
    }

    #[test]
    fn compressed_capability_selection_and_decode_keep_channels_and_partial_blocks() {
        let etc2 = vec![129, 0, 0, 0, 0, 0, 0, 0, 0xff, 0, 0, 0, 0, 0, 0, 0];
        for (format, block, expected) in [
            (ImageFormat::Astc4x4, astc(), [201, 81, 21, 129]),
            (ImageFormat::Bc7, bc7(), [201, 81, 21, 129]),
            (ImageFormat::Etc2Rgba, etc2, [255, 2, 2, 129]),
        ] {
            let source = image(format, block);
            let native = Upload::new(&source, COMPRESSION_FEATURES).unwrap();
            assert!(matches!(native.bytes, Cow::Borrowed(_)));
            assert!(!native.format.is_srgb());
            assert_eq!(storage_bytes(native.format, 3, 2), 16);
            assert_eq!(storage_bytes(native.format, 5, 7), 64);
            let decoded = Upload::new(&source, wgpu::Features::empty()).unwrap();
            assert!(matches!(decoded.bytes, Cow::Owned(_)));
            // Every decoded texel is an 8-bit code; natively, ASTC's UNORM
            // decode is not, so it evaluates the transfer function.
            assert_eq!(decoded.flags & !TRANSFER_TABLE, SOURCE | SRGB_PRIMARIES);
            assert_ne!(decoded.flags & TRANSFER_TABLE, 0);
            let astc = format == ImageFormat::Astc4x4;
            assert_eq!((native.flags & TRANSFER) != 0, astc);
            assert_eq!((native.flags & TRANSFER_TABLE) != 0, !astc);
            assert_eq!(decoded.format, wgpu::TextureFormat::Rgba8Unorm);
            assert_eq!(decoded.bytes.len(), 24);
            for pixel in decoded.bytes.as_chunks::<4>().0 {
                assert_eq!(*pixel, expected);
            }
        }
    }
}
