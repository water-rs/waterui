//! Registered CPU images, shared by retained paint operands. A replacement
//! of the same dimensions decodes over the pixels once the operands that
//! shared them are discarded.

use cherenkov::{ImageColorSpace, ImageFormat, ImageUpload, ResourceError};

#[derive(Debug)]
pub struct CpuImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Box<[[f32; 4]]>,
}

impl CpuImage {
    pub fn bytes(&self) -> u64 {
        u64::try_from(size_of_val(&*self.pixels)).expect("image allocation fits u64")
    }

    /// Decode once on registration. Alpha is unpremultiplied in the encoded
    /// domain before transfer/primary conversion, then premultiplied in P3.
    /// `Rgba16F` texels convert f16 -> f64 directly — never through 8 bits —
    /// and keep values outside `[0, 1]`.
    pub fn decode(image: &ImageUpload) -> Result<Self, ResourceError> {
        let count = Self::validate(image)?;
        let mut pixels = vec![[0.0; 4]; count].into_boxed_slice();
        convert(image, &mut pixels);
        Ok(Self {
            width: image.width,
            height: image.height,
            pixels,
        })
    }

    /// Decodes a validated `image` of this image's dimensions over the
    /// existing pixels, reusing their storage.
    pub fn overwrite(&mut self, image: &ImageUpload) {
        convert(image, &mut self.pixels);
    }

    /// Checks that `image` can be decoded: nonempty, a CPU-decodable
    /// format, and a byte length matching its dimensions. Returns its
    /// texel count.
    pub fn validate(image: &ImageUpload) -> Result<usize, ResourceError> {
        if image.width == 0 || image.height == 0 {
            return Err(ResourceError::Image(
                "CPU uploads require nonempty images".into(),
            ));
        }
        let bytes_per_texel = match image.format {
            ImageFormat::Rgba8 => 4,
            ImageFormat::Rgba16F => 8,
            format => {
                return Err(ResourceError::Image(format!(
                    "unsupported image format {format:?}"
                )));
            }
        };
        usize::try_from(image.width)
            .ok()
            .zip(usize::try_from(image.height).ok())
            .and_then(|(width, height)| width.checked_mul(height))
            .filter(|count| count.checked_mul(bytes_per_texel) == Some(image.data.len()))
            .ok_or_else(|| ResourceError::Image("image dimensions and byte length disagree".into()))
    }
}

/// Decodes a validated `image` into `pixels`, one working-space texel per
/// encoded texel.
#[expect(
    clippy::cast_possible_truncation,
    reason = "decoded working pixels use f32"
)]
fn convert(image: &ImageUpload, pixels: &mut [[f32; 4]]) {
    // u8 data can be premultiplied-encodable only up to 1.0; f16 texels
    // keep extended values, so their un-premultiply is unbounded.
    let unpremul_max = match image.format {
        ImageFormat::Rgba8 => 1.0,
        _ => f64::INFINITY,
    };
    let texel = |encoded: [f64; 4]| {
        let alpha = encoded[3];
        if alpha == 0.0 {
            return [0.0; 4];
        }
        let linear = std::array::from_fn(|channel| {
            let straight = if image.premultiplied {
                (encoded[channel] / alpha).min(unpremul_max)
            } else {
                encoded[channel]
            };
            if image.color_space == ImageColorSpace::LinearSrgb
                || image.color_space == ImageColorSpace::LinearP3
            {
                straight
            } else if straight <= 0.04045 {
                straight / 12.92
            } else {
                ((straight + 0.055) / 1.055).powf(2.4)
            }
        });
        let working = if image.color_space == ImageColorSpace::DisplayP3
            || image.color_space == ImageColorSpace::LinearP3
        {
            linear
        } else {
            mul(&XYZ_TO_P3, mul(&SRGB_TO_XYZ, linear))
        };
        [
            (working[0] * alpha) as f32,
            (working[1] * alpha) as f32,
            (working[2] * alpha) as f32,
            alpha as f32,
        ]
    };
    match image.format {
        ImageFormat::Rgba8 => {
            for (out, pixel) in pixels.iter_mut().zip(image.data.as_chunks::<4>().0) {
                *out = texel(pixel.map(|v| f64::from(v) / 255.0));
            }
        }
        ImageFormat::Rgba16F => {
            for (out, pixel) in pixels.iter_mut().zip(image.data.as_chunks::<8>().0) {
                *out = texel(std::array::from_fn(|channel| {
                    f64::from(half::f16::from_le_bytes([
                        pixel[2 * channel],
                        pixel[2 * channel + 1],
                    ]))
                }));
            }
        }
        _ => unreachable!("format validated before conversion"),
    }
}

const SRGB_TO_XYZ: [[f64; 3]; 3] = [
    [
        0.412_390_799_265_959_5,
        0.357_584_339_383_878,
        0.180_480_788_401_834_3,
    ],
    [
        0.212_639_005_871_510_4,
        0.715_168_678_767_756,
        0.072_192_315_360_733_7,
    ],
    [
        0.019_330_818_715_591_9,
        0.119_194_779_794_626,
        0.950_532_152_249_660_7,
    ],
];
const XYZ_TO_P3: [[f64; 3]; 3] = [
    [
        2.493_496_911_941_425,
        -0.931_383_617_919_123_9,
        -0.402_710_784_450_716_8,
    ],
    [
        -0.829_488_969_561_574_7,
        1.762_664_060_318_346_3,
        0.023_624_685_841_943_6,
    ],
    [
        0.035_845_830_243_784_5,
        -0.076_172_389_268_041_8,
        0.956_884_524_007_687_2,
    ],
];
fn mul(matrix: &[[f64; 3]; 3], value: [f64; 3]) -> [f64; 3] {
    matrix.map(|row| row[2].mul_add(value[2], row[1].mul_add(value[1], row[0] * value[0])))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `LinearP3` upload is the working space already: no transfer
    /// function and no primaries matrix — the byte value passes through.
    #[test]
    fn linear_p3_upload_decodes_as_identity() {
        let image = ImageUpload {
            width: 1,
            height: 1,
            data: vec![255, 0, 0, 255].into(),
            color_space: ImageColorSpace::LinearP3,
            premultiplied: false,
            format: ImageFormat::Rgba8,
        };
        let decoded = CpuImage::decode(&image).expect("decode");
        assert_eq!(decoded.pixels[0], [1.0, 0.0, 0.0, 1.0]);
    }
}
