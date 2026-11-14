//! Bitmap font strike selection and glyph decoding.

use kurbo::Rect;
use skrifa::bitmap::{BitmapData, BitmapFormat, BitmapGlyph, BitmapStrikes, Origin};
use skrifa::raw::TableProvider;

use cherenkov::{RenderError, ResourceError};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BitmapKey {
    pub font: u64,
    pub strike: u16,
    pub glyph: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Sbix,
    Cbdt,
}

#[derive(Debug)]
pub struct BitmapFont {
    pub format: Format,
    pub strikes: Box<[(f32, u16)]>,
}

impl BitmapFont {
    pub fn detect(data: &[u8], index: u32) -> Result<Option<Self>, ResourceError> {
        let font = skrifa::FontRef::from_index(data, index)
            .map_err(|error| ResourceError::Font(error.to_string()))?;
        let (format, bitmap_format) = if font.data_for_tag(skrifa::Tag::new(b"sbix")).is_some() {
            font.sbix()
                .map_err(|error| ResourceError::Font(error.to_string()))?;
            (Format::Sbix, BitmapFormat::Sbix)
        } else if font.data_for_tag(skrifa::Tag::new(b"CBDT")).is_some() {
            font.cblc()
                .map_err(|error| ResourceError::Font(error.to_string()))?;
            font.cbdt()
                .map_err(|error| ResourceError::Font(error.to_string()))?;
            (Format::Cbdt, BitmapFormat::Cbdt)
        } else {
            return Ok(None);
        };
        let strikes = BitmapStrikes::with_format(&font, bitmap_format)
            .ok_or_else(|| ResourceError::Font("invalid bitmap strike tables".into()))?;
        if strikes.is_empty() {
            return Err(ResourceError::Font("bitmap font has no strikes".into()));
        }
        let mut sizes = Vec::with_capacity(strikes.len());
        for (index, strike) in strikes.iter().enumerate() {
            let index = u16::try_from(index)
                .map_err(|_| ResourceError::Font("too many bitmap strikes".into()))?;
            sizes.push((strike.ppem(), index));
        }
        sizes.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        Ok(Some(Self {
            format,
            strikes: sizes.into_boxed_slice(),
        }))
    }

    pub fn select(&self, device_ppem: f32) -> u16 {
        if !device_ppem.is_finite() || device_ppem <= 0.0 {
            return self.largest_strike();
        }
        self.strikes
            .iter()
            .copied()
            .find(|(ppem, _)| *ppem >= device_ppem)
            .map_or_else(|| self.largest_strike(), |(_, index)| index)
    }

    fn largest_strike(&self) -> u16 {
        let largest_ppem = self.strikes.last().expect("bitmap font has strikes").0;
        self.strikes
            .iter()
            .find(|(ppem, _)| ppem.to_bits() == largest_ppem.to_bits())
            .expect("largest bitmap strike exists")
            .1
    }
}

#[derive(Debug)]
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub premultiplied: bool,
    pub em: Rect,
}

pub fn decode(
    data: &[u8],
    index: u32,
    bitmap_font: &BitmapFont,
    strike_index: u16,
    glyph_id: u32,
) -> Result<Option<Decoded>, RenderError> {
    let font = skrifa::FontRef::from_index(data, index)
        .map_err(|error| RenderError::Font(error.to_string()))?;
    let gid = skrifa::GlyphId::new(glyph_id);
    let strikes = BitmapStrikes::with_format(
        &font,
        match bitmap_font.format {
            Format::Sbix => BitmapFormat::Sbix,
            Format::Cbdt => BitmapFormat::Cbdt,
        },
    )
    .ok_or_else(|| RenderError::Font("invalid bitmap strike tables".into()))?;
    let strike = strikes
        .get(usize::from(strike_index))
        .ok_or_else(|| RenderError::Font(format!("bitmap strike {strike_index} is missing")))?;
    let glyph = if let Some(glyph) = strike.get(gid) {
        glyph
    } else {
        match bitmap_font.format {
            Format::Sbix => {
                let sbix = font
                    .sbix()
                    .map_err(|error| RenderError::Font(error.to_string()))?;
                let raw_strike = sbix
                    .strikes()
                    .get(usize::from(strike_index))
                    .map_err(|error| RenderError::Font(error.to_string()))?;
                let raw = match raw_strike.glyph_data(gid) {
                    Ok(Some(raw)) => raw,
                    Ok(None) | Err(skrifa::raw::ReadError::OutOfBounds) => return Ok(None),
                    Err(error) => return Err(RenderError::Font(error.to_string())),
                };
                if raw.graphic_type() == skrifa::Tag::new(b"dupe") {
                    let target = raw
                        .data()
                        .get(..2)
                        .ok_or_else(|| RenderError::Font("invalid sbix dupe glyph data".into()))?;
                    let target = u16::from_be_bytes([target[0], target[1]]);
                    let target = skrifa::GlyphId::from(target);
                    let target_raw = match raw_strike.glyph_data(target) {
                        Ok(Some(target_raw)) => target_raw,
                        Ok(None) | Err(skrifa::raw::ReadError::OutOfBounds) => {
                            return Err(RenderError::Font("sbix dupe target is absent".into()));
                        }
                        Err(error) => return Err(RenderError::Font(error.to_string())),
                    };
                    if target_raw.graphic_type() == skrifa::Tag::new(b"dupe") {
                        return Err(RenderError::Font("sbix dupe points to another dupe".into()));
                    }
                    if target_raw.graphic_type() != skrifa::Tag::new(b"png ") {
                        return Err(RenderError::Unsupported(crate::names::COLOR_FONT));
                    }
                    strike.get(target).ok_or_else(|| {
                        RenderError::Font("invalid sbix dupe target glyph metrics".into())
                    })?
                } else if raw.graphic_type() == skrifa::Tag::new(b"png ") {
                    return Err(RenderError::Font("invalid sbix PNG glyph metrics".into()));
                } else {
                    return Err(RenderError::Unsupported(crate::names::COLOR_FONT));
                }
            }
            Format::Cbdt => {
                let cblc = font
                    .cblc()
                    .map_err(|error| RenderError::Font(error.to_string()))?;
                let size = cblc
                    .bitmap_sizes()
                    .get(usize::from(strike_index))
                    .copied()
                    .ok_or_else(|| {
                        RenderError::Font(format!("bitmap strike {strike_index} is missing"))
                    })?;
                if size.location(cblc.offset_data(), gid).is_err() {
                    return Ok(None);
                }
                return Err(RenderError::Unsupported(crate::names::COLOR_FONT));
            }
        }
    };
    decode_glyph(&font, &glyph)
}

fn decode_glyph(
    font: &skrifa::FontRef<'_>,
    glyph: &BitmapGlyph<'_>,
) -> Result<Option<Decoded>, RenderError> {
    if glyph.width == 0 || glyph.height == 0 {
        return Ok(None);
    }
    let (rgba, premultiplied, image_width, image_height) = match &glyph.data {
        BitmapData::Png(bytes) => {
            let (rgba, width, height) = decode_png(bytes)?;
            (rgba, false, width, height)
        }
        BitmapData::Bgra(bytes) => {
            let expected = usize::try_from(glyph.width)
                .ok()
                .and_then(|width| {
                    usize::try_from(glyph.height)
                        .ok()
                        .and_then(|height| width.checked_mul(height))
                })
                .and_then(|pixels| pixels.checked_mul(4));
            if expected != Some(bytes.len()) {
                return Err(RenderError::Font(
                    "bitmap glyph image disagrees with its metrics".into(),
                ));
            }
            (
                bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .flat_map(|pixel| [pixel[2], pixel[1], pixel[0], pixel[3]])
                    .collect(),
                true,
                glyph.width,
                glyph.height,
            )
        }
        BitmapData::Mask(_) => {
            return Err(RenderError::Unsupported(crate::names::COLOR_FONT));
        }
    };
    if image_width != glyph.width || image_height != glyph.height {
        return Err(RenderError::Font(
            "bitmap glyph image disagrees with its metrics".into(),
        ));
    }
    let image_pixels = rgba.len() / 4;
    let metric_pixels = usize::try_from(glyph.width).ok().and_then(|width| {
        usize::try_from(glyph.height)
            .ok()
            .and_then(|height| width.checked_mul(height))
    });
    if metric_pixels != Some(image_pixels) {
        return Err(RenderError::Font(
            "bitmap glyph image disagrees with its metrics".into(),
        ));
    }
    let upem = font
        .head()
        .map_err(|error| RenderError::Font(error.to_string()))?
        .units_per_em();
    if upem == 0 || glyph.ppem_x <= 0.0 || glyph.ppem_y <= 0.0 {
        return Err(RenderError::Font("invalid bitmap glyph metrics".into()));
    }
    let x0 = f64::from(glyph.bearing_x) / f64::from(upem)
        + f64::from(glyph.inner_bearing_x) / f64::from(glyph.ppem_x);
    let y = f64::from(glyph.bearing_y) / f64::from(upem)
        + f64::from(glyph.inner_bearing_y) / f64::from(glyph.ppem_y);
    let width = f64::from(glyph.width) / f64::from(glyph.ppem_x);
    let height = f64::from(glyph.height) / f64::from(glyph.ppem_y);
    let (y0, y1) = match glyph.placement_origin {
        Origin::TopLeft => (-y, -y + height),
        Origin::BottomLeft => (-y - height, -y),
    };
    Ok(Some(Decoded {
        width: glyph.width,
        height: glyph.height,
        rgba,
        premultiplied,
        em: Rect::new(x0, y0, x0 + width, y1),
    }))
}

fn decode_png(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), RenderError> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|error| RenderError::Font(format!("bitmap PNG: {error}")))?;
    let mut out = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or_else(|| RenderError::Font("bitmap PNG has unknown size".into()))?
    ];
    let info = reader
        .next_frame(&mut out)
        .map_err(|error| RenderError::Font(format!("bitmap PNG: {error}")))?;
    out.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => out,
        png::ColorType::Rgb => out
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
            .collect(),
        png::ColorType::Grayscale => out
            .iter()
            .flat_map(|&gray| [gray, gray, gray, 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => out
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|pixel| [pixel[0], pixel[0], pixel[0], pixel[1]])
            .collect(),
        other @ png::ColorType::Indexed => {
            return Err(RenderError::Font(format!(
                "bitmap PNG has unexpected color type {other:?}"
            )));
        }
    };
    Ok((rgba, info.width, info.height))
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrifa::MetadataProvider;

    const CBDT: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../scenes/fonts/NotoColorEmojiSubset.ttf"
    ));
    const SBIX: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../scenes/fonts/CherenkovSbixTest.ttf"
    ));

    fn fixture_font() -> BitmapFont {
        BitmapFont {
            format: Format::Sbix,
            strikes: vec![(32.0, 0), (96.0, 1)].into_boxed_slice(),
        }
    }

    #[test]
    fn strike_selection_uses_the_smallest_large_enough_strike() {
        let font = fixture_font();
        assert_eq!(font.select(32.0), 0);
        assert_eq!(font.select(33.0), 1);
        assert_eq!(font.select(16.0), 0);
        assert_eq!(font.select(160.0), 1);
        assert_eq!(font.select(f32::NAN), 1);
        assert_eq!(font.select(f32::INFINITY), 1);
        assert_eq!(font.select(0.0), 1);
        assert_eq!(font.select(-1.0), 1);

        let tied = BitmapFont {
            format: Format::Sbix,
            strikes: vec![(32.0, 0), (32.0, 2), (96.0, 1), (96.0, 3)].into_boxed_slice(),
        };
        assert_eq!(tied.select(32.0), 0);
        assert_eq!(tied.select(97.0), 1);
        assert_eq!(tied.select(f32::NAN), 1);
    }

    #[test]
    fn detects_and_decodes_the_cbdt_subset() {
        let font = BitmapFont::detect(CBDT, 0)
            .expect("detect CBDT")
            .expect("bitmap font");
        assert_eq!(font.format, Format::Cbdt);
        assert_eq!(&*font.strikes, &[(109.0, 0)]);
        assert!(
            decode(CBDT, 0, &font, 0, 0)
                .expect("missing .notdef")
                .is_none()
        );

        let font_ref = skrifa::FontRef::from_index(CBDT, 0).expect("font");
        let glyph = font_ref.charmap().map('😀').expect("emoji glyph").to_u32();
        let decoded = decode(CBDT, 0, &font, 0, glyph)
            .expect("decode CBDT")
            .expect("glyph bitmap");
        assert_eq!((decoded.width, decoded.height), (136, 128));
        assert!((decoded.em.x0 - 0.0).abs() < 1e-12);
        assert!((decoded.em.y0 - (-101.0 / 109.0)).abs() < 1e-12);
        assert!((decoded.em.x1 - (136.0 / 109.0)).abs() < 1e-12);
        assert!((decoded.em.y1 - (27.0 / 109.0)).abs() < 1e-12);
    }

    #[test]
    fn detects_and_decodes_the_sbix_strikes() {
        let font = BitmapFont::detect(SBIX, 0)
            .expect("detect sbix")
            .expect("bitmap font");
        assert_eq!(font.format, Format::Sbix);
        assert_eq!(&*font.strikes, &[(32.0, 0), (96.0, 1)]);
        assert!(
            decode(SBIX, 0, &font, 0, 0)
                .expect("missing .notdef")
                .is_none()
        );

        let font_ref = skrifa::FontRef::from_index(SBIX, 0).expect("font");
        let glyph = font_ref.charmap().map('😀').expect("emoji glyph").to_u32();
        let decoded = decode(SBIX, 0, &font, 0, glyph)
            .expect("decode sbix")
            .expect("glyph bitmap");
        assert_eq!((decoded.width, decoded.height), (40, 38));
    }
}
