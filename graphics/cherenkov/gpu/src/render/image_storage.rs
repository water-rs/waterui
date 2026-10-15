//! Source storage and sampling metadata; compositing remains linear Display P3.
use cherenkov::{ImageColorSpace, ImageFormat, ImageUpload, ResourceError};
use std::borrow::Cow;

pub(super) const COMPRESSION_FEATURES: wgpu::Features = wgpu::Features::TEXTURE_COMPRESSION_ASTC
    .union(wgpu::Features::TEXTURE_COMPRESSION_BC)
    .union(wgpu::Features::TEXTURE_COMPRESSION_ETC2);

pub(super) struct Upload<'a> {
    pub bytes: Cow<'a, [u8]>,
    pub format: wgpu::TextureFormat,
    pub flags: u32,
}

impl<'a> Upload<'a> {
    pub fn new(image: &'a ImageUpload, features: wgpu::Features) -> Result<Self, ResourceError> {
        use wgpu::TextureFormat as T;
        let nonlinear = matches!(
            image.color_space,
            ImageColorSpace::Srgb | ImageColorSpace::DisplayP3
        );
        // sRGB texture decoding can quantize dark values more coarsely than
        // the working f16 representation. Preserve the encoded bytes and
        // decode each tap in the shader, for both alpha interpretations.
        let rgba8 = T::Rgba8Unorm;
        let (native, required) = match image.format {
            ImageFormat::Rgba8 => (rgba8, wgpu::Features::empty()),
            ImageFormat::Rgba16F => (T::Rgba16Float, wgpu::Features::empty()),
            ImageFormat::Astc4x4 => (
                T::Astc {
                    block: wgpu::AstcBlock::B4x4,
                    channel: wgpu::AstcChannel::Unorm,
                },
                wgpu::Features::TEXTURE_COMPRESSION_ASTC,
            ),
            ImageFormat::Bc7 => (
                T::Bc7RgbaUnorm,
                wgpu::Features::TEXTURE_COMPRESSION_BC,
            ),
            ImageFormat::Etc2Rgba => (
                T::Etc2Rgba8Unorm,
                wgpu::Features::TEXTURE_COMPRESSION_ETC2,
            ),
        };
        let (format, bytes) = if features.contains(required) {
            (native, Cow::Borrowed(image.data.as_ref()))
        } else {
            tracing::debug!(format = ?image.format, required = ?required, "decoding image because the device lacks its compression feature");
            (rgba8, Cow::Owned(decode(image)?))
        };
        // Image paint bits 12–16: source storage, premultiplied input,
        // shader transfer decode, sRGB primaries, unbounded f16 channels.
        // Zero is reserved for internal, already-linear-P3 textures.
        let flags = (1 << 12)
            | (u32::from(image.premultiplied) << 13)
            | (u32::from(nonlinear) << 14)
            | (u32::from(matches!(
                image.color_space,
                ImageColorSpace::Srgb | ImageColorSpace::LinearSrgb
            )) << 15)
            | (u32::from(image.format == ImageFormat::Rgba16F) << 16);
        Ok(Self {
            bytes,
            format,
            flags,
        })
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
    fn source_storage_borrows_bytes_and_preserves_color_metadata() {
        for color_space in [
            ImageColorSpace::Srgb,
            ImageColorSpace::DisplayP3,
            ImageColorSpace::LinearSrgb,
            ImageColorSpace::LinearP3,
        ] {
            for premultiplied in [false, true] {
                for format in [ImageFormat::Rgba8, ImageFormat::Rgba16F] {
                    let mut source = image(
                        format,
                        vec![0; if format == ImageFormat::Rgba8 { 24 } else { 48 }],
                    );
                    source.color_space = color_space;
                    source.premultiplied = premultiplied;
                    let upload = Upload::new(&source, wgpu::Features::empty()).unwrap();
                    assert!(matches!(upload.bytes, Cow::Borrowed(_)));
                    assert_eq!(upload.bytes.as_ptr(), source.data.as_ptr());
                    assert_eq!((upload.flags & 8192) != 0, premultiplied);
                    assert_eq!(
                        (upload.flags & 32768) != 0,
                        matches!(
                            color_space,
                            ImageColorSpace::Srgb | ImageColorSpace::LinearSrgb
                        )
                    );
                    assert_eq!((upload.flags & 65536) != 0, format == ImageFormat::Rgba16F);
                    let nonlinear = matches!(
                        color_space,
                        ImageColorSpace::Srgb | ImageColorSpace::DisplayP3
                    );
                    assert!(!upload.format.is_srgb());
                    assert_eq!(
                        (upload.flags & 16384) != 0,
                        nonlinear && !upload.format.is_srgb()
                    );
                    assert_eq!(storage_bytes(upload.format, 3, 2), source.data.len() as u64);
                }
            }
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
            assert_eq!(decoded.flags, native.flags);
            assert_eq!(decoded.format, wgpu::TextureFormat::Rgba8Unorm);
            assert_eq!(decoded.bytes.len(), 24);
            for pixel in decoded.bytes.as_chunks::<4>().0 {
                assert_eq!(*pixel, expected);
            }
        }
    }
}
