//! Pixel buffers: premultiplied linear Display P3 images in `f64`
//! (compositing space) and `f32` (output space), plus PNG writing.

/// An image of linear-light Display P3 RGBA, premultiplied, `f64` per channel.
///
/// The oracle composites in `f64` and converts to `f32` only for output.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    /// Width in pixels.
    pub width: usize,
    /// Height in pixels.
    pub height: usize,
    /// Row-major premultiplied RGBA, `f64` per channel.
    pub pixels: Vec<[f64; 4]>,
}

impl Image {
    /// A transparent image of `width`×`height`.
    #[must_use]
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            pixels: vec![[0.0; 4]; width * height],
        }
    }

    /// A uniformly `color` image.
    #[must_use]
    pub fn filled(width: usize, height: usize, color: [f64; 4]) -> Self {
        Self {
            width,
            height,
            pixels: vec![color; width * height],
        }
    }
}

/// Decode a PNG blob to straight-alpha `rgba8` pixels, handling every PNG
/// colour type (palette, greyscale, grey-alpha, RGB, RGBA, 1–16 bit).
///
/// `EXPAND` grows palette, sub-byte greyscale and `tRNS` data; `STRIP_16`
/// reduces 16-bit channels to 8. The resulting frame is RGBA8, RGB8,
/// greyscale-8 or grey-alpha-8 and is expanded to straight-alpha RGBA8
/// here.
///
/// This is the single PNG decoder shared by the oracle
/// ([`crate::resources`]) and the bench adapters.
///
/// # Errors
/// [`png::DecodingError`] on malformed input, `std::io::Error` on
/// unexpected short reads.
pub fn decode_png_rgba8(
    bytes: &[u8],
) -> Result<(u32, u32, Vec<u8>), Box<dyn std::error::Error + Send + Sync>> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let mut out = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or("png: unknown output size")?
    ];
    let info = reader.next_frame(&mut out)?;
    out.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => out,
        png::ColorType::Rgb => out
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::Grayscale => out.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => out
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        other @ png::ColorType::Indexed => {
            return Err(format!("png: unexpected output color type {other:?}").into());
        }
    };
    Ok((info.width, info.height, rgba))
}

/// An image of linear-light Display P3 RGBA, premultiplied, `f32` per
/// channel — the interchange format every bench adapter returns.
#[derive(Clone, Debug, PartialEq)]
pub struct F32Image {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major premultiplied RGBA, `f32` per channel.
    pub pixels: Vec<[f32; 4]>,
}

impl F32Image {
    /// A transparent image of `width`×`height`.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![[0.0; 4]; (width * height) as usize],
        }
    }

    /// Convert from the oracle's `f64` working image.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the f32 interchange format deliberately narrows the f64 pipeline"
    )]
    pub fn from_f64(image: &Image) -> Self {
        Self {
            width: image.width as u32,
            height: image.height as u32,
            pixels: image
                .pixels
                .iter()
                .map(|p| [p[0] as f32, p[1] as f32, p[2] as f32, p[3] as f32])
                .collect(),
        }
    }

    /// PNG-encode as gamma-encoded sRGB 8-bit RGBA (for viewing).
    ///
    /// Pixels are un-premultiplied, mapped from linear P3 to linear sRGB,
    /// gamma-encoded and clamped to `[0, 1]`; out-of-gamut values clip.
    ///
    /// # Errors
    /// Returns `std::io::Error` on file or encoding failure.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "channels are clamped to [0,1] before the deliberate u8 quantize"
    )]
    pub fn write_png(&self, path: &std::path::Path) -> std::io::Result<()> {
        let mut data = Vec::with_capacity(self.pixels.len() * 4);
        for p in &self.pixels {
            let a = f64::from(p[3].clamp(0.0, 1.0));
            let unpremul = if a > 0.0 {
                [
                    f64::from(p[0]) / a,
                    f64::from(p[1]) / a,
                    f64::from(p[2]) / a,
                ]
            } else {
                [0.0; 3]
            };
            let srgb = crate::color::linear_p3_to_linear_srgb(unpremul);
            for c in srgb {
                data.push((crate::color::srgb_encode(c).clamp(0.0, 1.0) * 255.0).round() as u8);
            }
            data.push((a * 255.0).round() as u8);
        }
        let file = std::fs::File::create(path)?;
        let mut writer = std::io::BufWriter::new(file);
        let mut encoder = png::Encoder::new(&mut writer, self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut png_writer = encoder.write_header()?;
        png_writer.write_image_data(&data)?;
        Ok(())
    }
}
