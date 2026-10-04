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
        let key = (hash, encoding);
        if !self.images.contains_key(&key) {
            let bytes = self.blob(hash)?;
            let invalid =
                |e: String| SceneError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e));
            let (width, height, pixels) = match encoding {
                ImageEncoding::Png { color_space } => {
                    let (width, height, rgba) = crate::image::decode_png_rgba8(&bytes)
                        .map_err(|e| invalid(e.to_string()))?;
                    let mut pixels = Vec::with_capacity(width as usize * height as usize);
                    for px in rgba.as_chunks::<4>().0 {
                        let a = f64::from(px[3]) / 255.0;
                        let srgb = [
                            srgb_decode(f64::from(px[0]) / 255.0),
                            srgb_decode(f64::from(px[1]) / 255.0),
                            srgb_decode(f64::from(px[2]) / 255.0),
                        ];
                        let lin_p3 = match color_space {
                            ImageColorSpace::Srgb => linear_srgb_to_linear_p3(srgb),
                            ImageColorSpace::DisplayP3 => srgb,
                            _ => return Err(SceneError::InvalidImageEncoding(encoding)),
                        };
                        pixels.push([a * lin_p3[0], a * lin_p3[1], a * lin_p3[2], a]);
                    }
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
