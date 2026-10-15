//! Decoded scene resources: PNG images become premultiplied linear
//! Display P3 `f64` images; font blobs stay as byte caches for `skrifa`.

use std::collections::HashMap;
use std::path::PathBuf;

use cherenkov_scene::{ResourceHash, Scene, SceneError};

use crate::color::{linear_srgb_to_linear_p3, srgb_decode};
use crate::filter::Texels;
use crate::image::Image;

/// Lazily decoded `resources/` blobs for one scene directory.
#[derive(Debug)]
pub struct Resources {
    dir: PathBuf,
    images: HashMap<(ResourceHash, cherenkov_scene::ImageEncoding), Image>,
    texels: HashMap<ResourceHash, Texels>,
    fonts: HashMap<ResourceHash, Vec<u8>>,
}

impl Resources {
    /// Resources under `scene_dir` (its `resources/` directory).
    #[must_use]
    pub fn new(scene_dir: PathBuf) -> Self {
        Self {
            dir: scene_dir,
            images: HashMap::new(),
            texels: HashMap::new(),
            fonts: HashMap::new(),
        }
    }

    fn blob(&self, hash: ResourceHash) -> Result<Vec<u8>, SceneError> {
        Scene::resource(&self.dir, hash)
    }

    /// Adds an already-decoded image, preserving an existing content entry.
    pub fn insert_image(&mut self, hash: ResourceHash, image: Image) {
        self.images
            .entry((hash, cherenkov_scene::ImageEncoding::default()))
            .or_insert(image);
    }

    /// The decoded image for `hash` under `encoding`, decoding on first use.
    ///
    /// `Png` blobs carry encoded 8-bit data (`Srgb` primaries, or `DisplayP3`
    /// which shares the sRGB transfer function); every PNG colour type is
    /// handled by [`crate::image::decode_png_rgba8`]. `Rgba16F` blobs are raw
    /// little-endian half floats, straight alpha, in linear light. sRGB
    /// primaries then convert to linear Display P3; P3 primaries pass
    /// through — into the premultiplied `f64` working image.
    ///
    /// # Errors
    /// [`SceneError::MissingResource`] if the blob is absent, [`SceneError::Io`]
    /// on read or decode failure, [`SceneError::InvalidImageEncoding`] if the
    /// blob length does not match the declared `Rgba16F` dimensions.
    /// # Panics
    /// Never — the cache entry was just inserted on the miss path.
    pub fn image(
        &mut self,
        hash: ResourceHash,
        encoding: cherenkov_scene::ImageEncoding,
    ) -> Result<&Image, SceneError> {
        use cherenkov_scene::{ImageColorSpace, ImageEncoding};
        encoding.validate()?;
        let key = (hash, encoding);
        if !self.images.contains_key(&key) {
            let bytes = self.blob(hash)?;
            let invalid =
                |e: String| SceneError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e));
            let (width, height, pixels) = match encoding {
                ImageEncoding::Png { color_space } => {
                    let (width, height, rgba) = crate::image::decode_png_rgba8(&bytes)
                        .map_err(|e| invalid(e.to_string()))?;
                    let pixels = rgba
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|pixel| rgba8_working(*pixel, color_space, false))
                        .collect();
                    (width, height, pixels)
                }
                ImageEncoding::BlockCompressed {
                    format,
                    width,
                    height,
                    color_space,
                    premultiplied,
                } => {
                    let packed = decode_blocks(&bytes, format, width, height)?;
                    let pixels = packed
                        .into_iter()
                        .map(|pixel| {
                            let [b, g, r, a] = pixel.to_le_bytes();
                            rgba8_working([r, g, b, a], color_space, premultiplied)
                        })
                        .collect();
                    (width, height, pixels)
                }
                ImageEncoding::Rgba16F {
                    width,
                    height,
                    color_space,
                } => {
                    let expected = width as usize * height as usize * 8;
                    if bytes.len() != expected {
                        return Err(invalid(format!(
                            "rgba16f blob is {} bytes, expected {expected}",
                            bytes.len()
                        )));
                    }
                    let mut pixels = Vec::with_capacity(width as usize * height as usize);
                    for texel in bytes.as_chunks::<8>().0 {
                        let f = |i: usize| {
                            f64::from(half::f16::from_le_bytes([texel[2 * i], texel[2 * i + 1]]))
                        };
                        let a = f(3);
                        let rgb = [f(0), f(1), f(2)];
                        let lin_p3 = match color_space {
                            ImageColorSpace::LinearSrgb => linear_srgb_to_linear_p3(rgb),
                            ImageColorSpace::LinearP3 => rgb,
                            _ => return Err(SceneError::InvalidImageEncoding(encoding)),
                        };
                        pixels.push([a * lin_p3[0], a * lin_p3[1], a * lin_p3[2], a]);
                    }
                    (width, height, pixels)
                }
            };
            self.images.insert(
                key,
                Image {
                    width: width as usize,
                    height: height as usize,
                    pixels,
                },
            );
        }
        Ok(self.images.get(&key).unwrap())
    }

    /// The raw straight-alpha texels of filter image `hash`, `channel /
    /// 255` with no colour conversion, decoding on first use.
    ///
    /// # Errors
    /// As [`Self::image`].
    /// # Panics
    /// Never — the cache entry was just inserted on the miss path.
    pub fn texels(&mut self, hash: ResourceHash) -> Result<&Texels, SceneError> {
        if !self.texels.contains_key(&hash) {
            let bytes = self.blob(hash)?;
            let (width, height, rgba) = crate::image::decode_png_rgba8(&bytes).map_err(|e| {
                SceneError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    e.to_string(),
                ))
            })?;
            let texels = rgba
                .as_chunks::<4>()
                .0
                .iter()
                .map(|px| px.map(|v| f64::from(v) / 255.0))
                .collect();
            self.texels.insert(
                hash,
                Texels {
                    width: width as usize,
                    height: height as usize,
                    texels,
                },
            );
        }
        Ok(self.texels.get(&hash).unwrap())
    }

    /// The font bytes for `hash`, loading on first use.
    ///
    /// # Errors
    /// [`SceneError`] on missing/unreadable resource.
    /// # Panics
    /// Never — the cache entry was just inserted on the miss path.
    pub fn font(&mut self, hash: ResourceHash) -> Result<&Vec<u8>, SceneError> {
        if !self.fonts.contains_key(&hash) {
            let bytes = self.blob(hash)?;
            self.fonts.insert(hash, bytes);
        }
        Ok(self.fonts.get(&hash).unwrap())
    }
}

/// Convert each decoded source texel independently; interpolation belongs
/// to the oracle sampler and must run after this alpha/transfer conversion.
fn rgba8_working(
    pixel: [u8; 4],
    color_space: cherenkov_scene::ImageColorSpace,
    premultiplied: bool,
) -> [f64; 4] {
    use cherenkov_scene::ImageColorSpace;
    let [r, g, b, a] = pixel;
    let alpha = f64::from(a) / 255.0;
    let mut rgb = [r, g, b].map(|c| f64::from(c) / 255.0);
    if premultiplied && alpha > 0.0 {
        rgb = rgb.map(|c| (c / alpha).min(1.0));
    }
    if matches!(
        color_space,
        ImageColorSpace::Srgb | ImageColorSpace::DisplayP3
    ) {
        rgb = rgb.map(srgb_decode);
    }
    if matches!(
        color_space,
        ImageColorSpace::Srgb | ImageColorSpace::LinearSrgb
    ) {
        rgb = linear_srgb_to_linear_p3(rgb);
    }
    [rgb[0] * alpha, rgb[1] * alpha, rgb[2] * alpha, alpha]
}

fn decode_blocks(
    bytes: &[u8],
    format: cherenkov_scene::ImageCompression,
    width: u32,
    height: u32,
) -> Result<Vec<u32>, SceneError> {
    let invalid =
        |e: String| SceneError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e));
    let expected = format
        .byte_len(width, height)
        .expect("validated compressed dimensions");
    if bytes.len() != expected {
        return Err(invalid(format!(
            "{format:?} blob is {} bytes, expected {expected}",
            bytes.len()
        )));
    }
    let mut packed = vec![0_u32; width as usize * height as usize];
    match format {
        cherenkov_scene::ImageCompression::Astc4x4 => {
            texture2ddecoder::decode_astc(bytes, width as usize, height as usize, 4, 4, &mut packed)
        }
        cherenkov_scene::ImageCompression::Bc7 => {
            texture2ddecoder::decode_bc7(bytes, width as usize, height as usize, &mut packed)
        }
        cherenkov_scene::ImageCompression::Etc2Rgba => {
            texture2ddecoder::decode_etc2_rgba8(bytes, width as usize, height as usize, &mut packed)
        }
    }
    .map_err(|e| invalid(e.to_string()))?;
    Ok(packed)
}
