use alloc::{format, string::String, vec::Vec};
use core::convert::TryFrom;
use core::fmt;
use std::io::{Cursor, Seek, SeekFrom};

use mp4_atom::{Atom, Encode, FourCC, Ftyp, Header, Iinf, Meta, Pitm, ReadAtom, ReadFrom};

/// Decode route selected for a successfully decoded image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodePath {
    /// Use the platform-native decoder path.
    Platform,
    /// Fall back to software decoding in Rust.
    SoftwareFallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DecodeRoute {
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

#[derive(Debug, Clone)]
struct HeifContainerInfo {
    ftyp: Ftyp,
    ftyp_atom_len: usize,
    primary_item_type: Option<FourCC>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeifPrimaryCodec {
    Av1,
    Hevc,
    Other(FourCC),
    Missing,
}

const AVIF_BRAND: FourCC = FourCC::new(b"avif");
const AVIS_BRAND: FourCC = FourCC::new(b"avis");
const MIF1_BRAND: FourCC = FourCC::new(b"mif1");
const MSF1_BRAND: FourCC = FourCC::new(b"msf1");
const HEIF_BRAND: FourCC = FourCC::new(b"heif");
const HEIC_BRAND: FourCC = FourCC::new(b"heic");
const HEIX_BRAND: FourCC = FourCC::new(b"heix");
const HEVC_BRAND: FourCC = FourCC::new(b"hevc");
const HEVX_BRAND: FourCC = FourCC::new(b"hevx");
const AV01_ITEM_TYPE: FourCC = FourCC::new(b"av01");
const HVC1_ITEM_TYPE: FourCC = FourCC::new(b"hvc1");
const HEV1_ITEM_TYPE: FourCC = FourCC::new(b"hev1");

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

/// Decode an image with the software stack, retrying HEIF inputs through an AVIF-compatible
/// brand patch when necessary.
///
/// # Errors
///
/// Returns the original decode error when the primary decode and HEIF compatibility retry both fail.
pub fn decode_dynamic_image_with_heif_fallback(
    data: &[u8],
) -> Result<image::DynamicImage, image::ImageError> {
    match image::load_from_memory(data) {
        Ok(image) => Ok(image),
        Err(primary_err) => {
            let Ok(Some(patched)) = patch_heif_brand_to_avif(data) else {
                return Err(primary_err);
            };
            image::load_from_memory(&patched).map_err(|_| primary_err)
        }
    }
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
                    format!(
                        "Platform decode failed: {platform_err}; software fallback failed: \
                         {software_err}"
                    )
                }),
        },
        DecodeRoute::Software => decode_with_software_fallback(data)
            .map(|decoded| (decoded, DecodePath::SoftwareFallback))
            .map_err(|e| format!("Software decode failed: {e}")),
    }
}

pub(crate) fn detect_decode_route(data: &[u8]) -> DecodeRoute {
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
            ::image::ImageFormat::Avif if platform_available => DecodeRoute::Platform,
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
    match parse_heif_container(data) {
        Ok(Some(_)) => true,
        Ok(None) | Err(_) => false,
    }
}

pub(crate) fn decode_with_software_fallback(data: &[u8]) -> Result<DecodedRgba, String> {
    let decoded = match waterkit_codec::decode_image(data) {
        Ok(decoded) => decoded,
        Err(primary_err) => {
            let patched = match patch_heif_brand_to_avif(data) {
                Ok(Some(patched)) => patched,
                Ok(None) => {
                    return Err(software_decode_error_message(data, &primary_err));
                }
                Err(parse_err) => {
                    return Err(format!(
                        "Image decode failed: {primary_err}; HEIF container parse failed: \
                         {parse_err}"
                    ));
                }
            };
            waterkit_codec::decode_image(&patched).map_err(|fallback_err| {
                format!("Image decode failed: {primary_err}; HEIF AV1 retry failed: {fallback_err}")
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

pub(crate) fn patch_heif_brand_to_avif(data: &[u8]) -> Result<Option<Vec<u8>>, String> {
    let Some(info) = parse_heif_container(data)? else {
        return Ok(None);
    };

    if heif_primary_codec(info.primary_item_type) != HeifPrimaryCodec::Av1 {
        return Ok(None);
    }

    let mut patched_ftyp = info.ftyp.clone();
    patched_ftyp.major_brand = AVIF_BRAND;
    ensure_compatible_brand(&mut patched_ftyp.compatible_brands, AVIF_BRAND);

    let mut encoded_ftyp = Vec::new();
    patched_ftyp
        .encode(&mut encoded_ftyp)
        .map_err(|error| format!("failed to encode patched ftyp: {error}"))?;
    if encoded_ftyp.len() != info.ftyp_atom_len {
        return Err(format!(
            "patched ftyp length changed from {} to {}",
            info.ftyp_atom_len,
            encoded_ftyp.len()
        ));
    }

    let mut patched = Vec::with_capacity(data.len());
    patched.extend_from_slice(&encoded_ftyp);
    patched.extend_from_slice(&data[info.ftyp_atom_len..]);
    Ok(Some(patched))
}

fn software_decode_error_message(data: &[u8], primary_err: &impl fmt::Display) -> String {
    match parse_heif_container(data) {
        Ok(Some(info)) => match heif_primary_codec(info.primary_item_type) {
            HeifPrimaryCodec::Av1 => {
                format!("Image decode failed: {primary_err}; HEIF AV1 retry was unavailable")
            }
            HeifPrimaryCodec::Hevc => format!(
                "Image decode failed: {primary_err}. HEIF software decode only supports AV1 \
                 payloads; primary item type is hvc1/hev1"
            ),
            HeifPrimaryCodec::Other(item_type) => format!(
                "Image decode failed: {primary_err}. HEIF software decode only supports AV1 \
                 payloads; primary item type is {item_type}"
            ),
            HeifPrimaryCodec::Missing => format!(
                "Image decode failed: {primary_err}. HEIF container is missing a primary image \
                 item type"
            ),
        },
        Ok(None) => format!("Image decode failed: {primary_err}"),
        Err(parse_err) => {
            format!("Image decode failed: {primary_err}; HEIF container parse failed: {parse_err}")
        }
    }
}

fn parse_heif_container(data: &[u8]) -> Result<Option<HeifContainerInfo>, String> {
    if !looks_like_bmff(data) {
        return Ok(None);
    }

    let mut cursor = Cursor::new(data);
    let header = Header::read_from(&mut cursor)
        .map_err(|error| format!("failed to read BMFF header: {error}"))?;
    if header.kind != Ftyp::KIND {
        return Ok(None);
    }

    let ftyp = Ftyp::read_atom(&header, &mut cursor)
        .map_err(|error| format!("failed to read ftyp: {error}"))?;
    if !ftyp_contains_heif_brand(&ftyp) {
        return Ok(None);
    }

    let ftyp_atom_len = usize::try_from(cursor.position())
        .map_err(|_| "ftyp offset does not fit into usize".to_string())?;
    let primary_item_type = parse_primary_item_type(&mut cursor)?;
    Ok(Some(HeifContainerInfo {
        ftyp,
        ftyp_atom_len,
        primary_item_type,
    }))
}

fn parse_primary_item_type(cursor: &mut Cursor<&[u8]>) -> Result<Option<FourCC>, String> {
    while let Some(header) = <Option<Header> as ReadFrom>::read_from(cursor)
        .map_err(|error| format!("failed to read BMFF atom header: {error}"))?
    {
        if header.kind == Meta::KIND {
            let meta = Meta::read_atom(&header, cursor)
                .map_err(|error| format!("failed to read meta atom: {error}"))?;
            let primary_item_id = meta.get::<Pitm>().map(|pitm| pitm.item_id);
            let item_infos = meta.get::<Iinf>().map(|iinf| &iinf.item_infos);
            return Ok(primary_item_id.and_then(|item_id| {
                item_infos.and_then(|infos| {
                    infos
                        .iter()
                        .find(|info| info.item_id == item_id)
                        .and_then(|info| info.item_type)
                })
            }));
        }

        let Some(size) = header.size else {
            return Ok(None);
        };
        let offset = i64::try_from(size).map_err(|_| "atom size exceeds i64".to_string())?;
        cursor
            .seek(SeekFrom::Current(offset))
            .map_err(|error| format!("failed to skip BMFF atom: {error}"))?;
    }

    Ok(None)
}

fn looks_like_bmff(data: &[u8]) -> bool {
    data.len() >= 8 && &data[4..8] == b"ftyp"
}

fn ftyp_contains_heif_brand(ftyp: &Ftyp) -> bool {
    is_heif_brand(ftyp.major_brand) || ftyp.compatible_brands.iter().copied().any(is_heif_brand)
}

const fn is_heif_brand(brand: FourCC) -> bool {
    matches!(
        brand,
        MIF1_BRAND | MSF1_BRAND | HEIF_BRAND | HEIC_BRAND | HEIX_BRAND | HEVC_BRAND | HEVX_BRAND
    )
}

const fn heif_primary_codec(primary_item_type: Option<FourCC>) -> HeifPrimaryCodec {
    match primary_item_type {
        Some(AV01_ITEM_TYPE) => HeifPrimaryCodec::Av1,
        Some(HVC1_ITEM_TYPE | HEV1_ITEM_TYPE) => HeifPrimaryCodec::Hevc,
        Some(other) => HeifPrimaryCodec::Other(other),
        None => HeifPrimaryCodec::Missing,
    }
}

fn ensure_compatible_brand(brands: &mut Vec<FourCC>, required: FourCC) {
    if brands
        .iter()
        .any(|brand| *brand == required || *brand == AVIS_BRAND)
    {
        return;
    }

    if let Some(index) = brands.iter().position(|brand| is_heif_brand(*brand)) {
        brands[index] = required;
    } else {
        brands.push(required);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use mp4_atom::{Any, Hdlr, ItemInfoEntry};

    fn build_heif_bytes(primary_item_type: FourCC) -> Vec<u8> {
        let ftyp = Ftyp {
            major_brand: HEIC_BRAND,
            minor_version: 0,
            compatible_brands: vec![MIF1_BRAND, HEIC_BRAND],
        };
        let meta = Meta {
            hdlr: Hdlr {
                handler: FourCC::new(b"pict"),
                name: String::new(),
            },
            items: vec![
                Any::from(Pitm { item_id: 1 }),
                Any::from(Iinf {
                    item_infos: vec![ItemInfoEntry {
                        item_id: 1,
                        item_protection_index: 0,
                        item_type: Some(primary_item_type),
                        item_name: String::new(),
                        content_type: None,
                        content_encoding: None,
                        item_not_in_presentation: false,
                    }],
                }),
            ],
        };

        let mut encoded = Vec::new();
        ftyp.encode(&mut encoded).expect("ftyp should encode");
        meta.encode(&mut encoded).expect("meta should encode");
        encoded
    }

    #[test]
    fn rewrites_heif_av1_ftyp_to_avif() {
        let encoded = build_heif_bytes(AV01_ITEM_TYPE);
        let patched = patch_heif_brand_to_avif(&encoded)
            .expect("heif parse should succeed")
            .expect("av1 heif should be patchable");

        let mut cursor = Cursor::new(patched.as_slice());
        let header = Header::read_from(&mut cursor).expect("header should decode");
        let ftyp = Ftyp::read_atom(&header, &mut cursor).expect("ftyp should decode");

        assert_eq!(ftyp.major_brand, AVIF_BRAND);
        assert!(ftyp.compatible_brands.contains(&AVIF_BRAND));
    }

    #[test]
    fn does_not_rewrite_heif_hevc_ftyp() {
        let encoded = build_heif_bytes(HVC1_ITEM_TYPE);
        let patched = patch_heif_brand_to_avif(&encoded).expect("heif parse should succeed");
        assert!(
            patched.is_none(),
            "HEVC-backed HEIF must stay on platform decode"
        );
    }
}
