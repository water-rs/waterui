use alloc::vec::Vec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodePath {
    Platform,
    SoftwareFallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecodeRoute {
    Platform,
    Software,
}

#[derive(Debug, Clone)]
pub(crate) struct DecodedRgba {
    pub(crate) pixels: Vec<u8>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) pixel_format: waterkit_codec::DecodedPixelFormat,
    pub(crate) hdr: bool,
    pub(crate) wide_gamut: bool,
}

pub(crate) fn decode_progressive_frame(data: &[u8]) -> Option<DecodedRgba> {
    match detect_decode_route(data) {
        DecodeRoute::Software => decode_with_software_fallback(data).ok(),
        DecodeRoute::Platform => None,
    }
}

pub(crate) fn is_progressive_candidate(content_type: Option<&str>, data: &[u8]) -> bool {
    if let Ok(format) = ::image::guess_format(data) {
        return matches!(
            format,
            ::image::ImageFormat::Jpeg
                | ::image::ImageFormat::Png
                | ::image::ImageFormat::Gif
                | ::image::ImageFormat::WebP
                | ::image::ImageFormat::Bmp
                | ::image::ImageFormat::Ico
                | ::image::ImageFormat::Tiff
        );
    }

    let Some(content_type) = content_type else {
        return false;
    };
    let lower = content_type.to_ascii_lowercase();
    lower.contains("image/jpeg")
        || lower.contains("image/png")
        || lower.contains("image/gif")
        || lower.contains("image/webp")
        || lower.contains("image/bmp")
        || lower.contains("image/x-icon")
        || lower.contains("image/vnd.microsoft.icon")
        || lower.contains("image/tiff")
}

pub(crate) fn decode_to_rgba8(data: &[u8]) -> Result<DecodedRgba, String> {
    decode_to_rgba8_with_path(data).map(|(decoded, _)| decoded)
}

pub(crate) fn decode_to_rgba8_with_path(data: &[u8]) -> Result<(DecodedRgba, DecodePath), String> {
    match detect_decode_route(data) {
        DecodeRoute::Platform => match decode_with_platform(data) {
            Ok(decoded) => Ok((decoded, DecodePath::Platform)),
            Err(platform_err) => decode_with_software_fallback(data)
                .map(|decoded| (decoded, DecodePath::SoftwareFallback))
                .map_err(|software_err| {
                    alloc::format!(
                        "Platform decode failed: {platform_err}; software fallback failed: \
                         {software_err}"
                    )
                }),
        },
        DecodeRoute::Software => decode_with_software_fallback(data)
            .map(|decoded| (decoded, DecodePath::SoftwareFallback))
            .map_err(|e| alloc::format!("Software decode failed: {e}")),
    }
}

fn detect_decode_route(data: &[u8]) -> DecodeRoute {
    let platform_available = cfg!(any(target_vendor = "apple", target_os = "android"));

    if is_heif_family(data) {
        return if platform_available {
            DecodeRoute::Platform
        } else {
            DecodeRoute::Software
        };
    }

    if let Ok(format) = ::image::guess_format(data) {
        let force_platform_for_color = platform_available
            && matches!(
                format,
                ::image::ImageFormat::Jpeg | ::image::ImageFormat::Png
            )
            && has_embedded_color_profile_hint(format, data);
        if force_platform_for_color {
            return DecodeRoute::Platform;
        }

        return match format {
            ::image::ImageFormat::Avif => {
                if platform_available {
                    DecodeRoute::Platform
                } else {
                    DecodeRoute::Software
                }
            }
            ::image::ImageFormat::Jpeg
            | ::image::ImageFormat::Png
            | ::image::ImageFormat::Gif
            | ::image::ImageFormat::WebP
            | ::image::ImageFormat::Bmp
            | ::image::ImageFormat::Ico
            | ::image::ImageFormat::Tiff => DecodeRoute::Software,
            _ => DecodeRoute::Software,
        };
    }

    DecodeRoute::Software
}

fn has_embedded_color_profile_hint(format: ::image::ImageFormat, data: &[u8]) -> bool {
    match format {
        ::image::ImageFormat::Png => png_has_color_profile_hint(data),
        ::image::ImageFormat::Jpeg => jpeg_has_icc_profile(data),
        _ => false,
    }
}

fn png_has_color_profile_hint(data: &[u8]) -> bool {
    const PNG_SIG: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if data.len() < 8 || &data[0..8] != PNG_SIG {
        return false;
    }

    let mut offset = 8usize;
    while offset + 12 <= data.len() {
        let len = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        let chunk_start = offset + 8;
        let chunk_end = chunk_start.saturating_add(len);
        if chunk_end + 4 > data.len() {
            return false;
        }
        let chunk_type = &data[offset + 4..offset + 8];
        if chunk_type == b"iCCP" {
            return true;
        }
        if chunk_type == b"cICP" && len == 4 {
            let primaries = data[chunk_start];
            let transfer = data[chunk_start + 1];
            if primaries != 1 || matches!(transfer, 16 | 18) {
                return true;
            }
        }
        if chunk_type == b"IEND" {
            break;
        }
        offset = chunk_end + 4;
    }
    false
}

fn jpeg_has_icc_profile(data: &[u8]) -> bool {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return false;
    }
    let mut i = 2usize;
    while i + 4 <= data.len() {
        if data[i] != 0xFF {
            break;
        }
        let marker = data[i + 1];
        i += 2;
        if marker == 0xD9 || marker == 0xDA {
            break;
        }
        if i + 2 > data.len() {
            break;
        }
        let seg_len = u16::from_be_bytes([data[i], data[i + 1]]) as usize;
        if seg_len < 2 || i + seg_len > data.len() {
            break;
        }
        let seg_data_start = i + 2;
        let seg_data_end = i + seg_len;
        if marker == 0xE2 {
            let seg_data = &data[seg_data_start..seg_data_end];
            if seg_data.starts_with(b"ICC_PROFILE\0") {
                return true;
            }
        }
        i += seg_len;
    }
    false
}

fn is_heif_family(data: &[u8]) -> bool {
    let Some(ftyp) = parse_ftyp(data) else {
        return false;
    };

    is_heif_brand(&ftyp.major) || ftyp.compat.iter().any(is_heif_brand)
}

#[derive(Debug, Clone)]
struct Ftyp {
    major: [u8; 4],
    compat: Vec<[u8; 4]>,
}

fn parse_ftyp(data: &[u8]) -> Option<Ftyp> {
    if data.len() < 16 {
        return None;
    }
    let box_size = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
    if box_size < 16 || box_size > data.len() || &data[4..8] != b"ftyp" {
        return None;
    }

    let major = [data[8], data[9], data[10], data[11]];
    let compat_bytes = &data[16..box_size];
    if compat_bytes.len() % 4 != 0 {
        return None;
    }
    let mut compat = Vec::with_capacity(compat_bytes.len() / 4);
    for chunk in compat_bytes.chunks_exact(4) {
        compat.push([chunk[0], chunk[1], chunk[2], chunk[3]]);
    }
    Some(Ftyp { major, compat })
}

pub(crate) fn decode_with_software_fallback(data: &[u8]) -> Result<DecodedRgba, String> {
    let decoded = match waterkit_codec::decode_image(data) {
        Ok(decoded) => decoded,
        Err(primary_err) => {
            let Some(patched) = patch_heif_brand_to_avif(data) else {
                return Err(alloc::format!("Image decode failed: {primary_err}"));
            };
            waterkit_codec::decode_image(&patched).map_err(|fallback_err| {
                alloc::format!(
                    "Image decode failed: {primary_err}; HEIF fallback failed: \
                     {fallback_err}. HEIF software decode only supports AV1 payloads"
                )
            })?
        }
    };
    let width = decoded.width();
    let height = decoded.height();
    let pixel_format = decoded.pixel_format();
    let hdr = decoded.hdr();
    let wide_gamut = decoded.wide_gamut();
    let pixels = decoded.into_pixels();
    Ok(DecodedRgba {
        pixels,
        width,
        height,
        pixel_format,
        hdr,
        wide_gamut,
    })
}

pub(crate) fn decode_with_platform(data: &[u8]) -> Result<DecodedRgba, String> {
    let decoded = waterkit_codec::decode_image_platform(data).map_err(|e| e.to_string())?;
    let width = decoded.width();
    let height = decoded.height();
    let pixel_format = decoded.pixel_format();
    let hdr = decoded.hdr();
    let wide_gamut = decoded.wide_gamut();
    let pixels = decoded.into_pixels();
    Ok(DecodedRgba {
        pixels,
        width,
        height,
        pixel_format,
        hdr,
        wide_gamut,
    })
}

fn is_generic_heif_brand(brand: &[u8; 4]) -> bool {
    matches!(brand, b"mif1" | b"msf1" | b"heif")
}

pub(crate) fn patch_heif_brand_to_avif(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 16 {
        return None;
    }
    let box_size = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
    if box_size < 16 || box_size > data.len() || &data[4..8] != b"ftyp" {
        return None;
    }

    let major = [data[8], data[9], data[10], data[11]];
    let is_heif = is_heif_brand(&major)
        || data[16..box_size]
            .chunks_exact(4)
            .any(|brand| is_heif_brand(&[brand[0], brand[1], brand[2], brand[3]]));
    if !is_heif {
        return None;
    }

    let mut patched = data.to_vec();
    patched[8..12].copy_from_slice(b"avif");

    let mut has_avif_compat = false;
    let mut first_heif_compat_offset: Option<usize> = None;
    for offset in (16..box_size).step_by(4) {
        if offset + 4 > box_size {
            break;
        }
        let brand = &patched[offset..offset + 4];
        if brand == b"avif" || brand == b"avis" {
            has_avif_compat = true;
            break;
        }
        if first_heif_compat_offset.is_none()
            && is_heif_brand(&[brand[0], brand[1], brand[2], brand[3]])
        {
            first_heif_compat_offset = Some(offset);
        }
    }
    if !has_avif_compat {
        if let Some(offset) = first_heif_compat_offset {
            patched[offset..offset + 4].copy_from_slice(b"avif");
        } else if box_size >= 20 {
            patched[16..20].copy_from_slice(b"avif");
        }
    }

    Some(patched)
}

fn is_heif_brand(brand: &[u8; 4]) -> bool {
    is_generic_heif_brand(brand) || matches!(brand, b"heic" | b"heix" | b"hevc" | b"hevx")
}
