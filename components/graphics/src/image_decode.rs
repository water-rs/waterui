extern crate alloc;

use alloc::vec::Vec;
use core::ops::Range;
use std::io::Cursor;

use mp4parse::{ParseStrictness, read_avif};

const FTYP_TYPE: &[u8; 4] = b"ftyp";
const AVIF_BRAND: [u8; 4] = *b"avif";
const AVIS_BRAND: [u8; 4] = *b"avis";

pub fn load_dynamic_image(bytes: &[u8]) -> Result<image::DynamicImage, image::ImageError> {
    match image::load_from_memory(bytes) {
        Ok(image) => Ok(image),
        Err(primary_err) => {
            let Some(patched) = bridge_heif_av1_to_avif(bytes) else {
                return Err(primary_err);
            };
            image::load_from_memory(&patched).map_err(|_| primary_err)
        }
    }
}

pub fn is_heif_family(data: &[u8]) -> bool {
    let Some(ftyp) = parse_file_type_box(data) else {
        return false;
    };

    is_heif_brand(&ftyp.major_brand)
        || ftyp
            .compatible_brands
            .iter()
            .any(|brand| is_heif_brand(brand))
}

pub fn bridge_heif_av1_to_avif(data: &[u8]) -> Option<Vec<u8>> {
    let ftyp = parse_file_type_box(data)?;
    if !is_heif_brand(&ftyp.major_brand)
        && !ftyp
            .compatible_brands
            .iter()
            .any(|brand| is_heif_brand(brand))
    {
        return None;
    }

    let mut patched = data.to_vec();
    patched[ftyp.major_brand_range].copy_from_slice(&AVIF_BRAND);

    let mut has_avif_compat = false;
    let mut first_heif_compat: Option<Range<usize>> = None;
    for range in &ftyp.compatible_brand_ranges {
        let brand = &patched[range.clone()];
        if brand == AVIF_BRAND || brand == AVIS_BRAND {
            has_avif_compat = true;
            break;
        }
        if first_heif_compat.is_none() && is_heif_brand(&[brand[0], brand[1], brand[2], brand[3]]) {
            first_heif_compat = Some(range.clone());
        }
    }
    if !has_avif_compat {
        if let Some(range) = first_heif_compat {
            patched[range].copy_from_slice(&AVIF_BRAND);
        } else if let Some(range) = ftyp.compatible_brand_ranges.first() {
            patched[range.clone()].copy_from_slice(&AVIF_BRAND);
        } else {
            return None;
        }
    }

    validate_avif_container(&patched).then_some(patched)
}

fn validate_avif_container(data: &[u8]) -> bool {
    read_avif(&mut Cursor::new(data), ParseStrictness::Normal).is_ok()
}

#[derive(Debug)]
struct FileTypeBox {
    major_brand: [u8; 4],
    major_brand_range: Range<usize>,
    compatible_brands: Vec<[u8; 4]>,
    compatible_brand_ranges: Vec<Range<usize>>,
}

fn parse_file_type_box(data: &[u8]) -> Option<FileTypeBox> {
    if data.len() < 20 {
        return None;
    }
    let box_size = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
    if box_size < 20 || box_size > data.len() || &data[4..8] != FTYP_TYPE {
        return None;
    }

    let major_brand_range = 8..12;
    let major_brand = [
        data[major_brand_range.start],
        data[major_brand_range.start + 1],
        data[major_brand_range.start + 2],
        data[major_brand_range.start + 3],
    ];

    let compatible_bytes = &data[16..box_size];
    if compatible_bytes.len() % 4 != 0 {
        return None;
    }

    let mut compatible_brands = Vec::with_capacity(compatible_bytes.len() / 4);
    let mut compatible_brand_ranges = Vec::with_capacity(compatible_bytes.len() / 4);
    for offset in (16..box_size).step_by(4) {
        let range = offset..offset + 4;
        compatible_brands.push([
            data[range.start],
            data[range.start + 1],
            data[range.start + 2],
            data[range.start + 3],
        ]);
        compatible_brand_ranges.push(range);
    }

    Some(FileTypeBox {
        major_brand,
        major_brand_range,
        compatible_brands,
        compatible_brand_ranges,
    })
}

fn is_heif_brand(brand: &[u8; 4]) -> bool {
    matches!(
        brand,
        b"mif1" | b"msf1" | b"heif" | b"heic" | b"heix" | b"hevc" | b"hevx"
    )
}
