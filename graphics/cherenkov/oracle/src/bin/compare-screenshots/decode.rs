use std::{ffi::c_void, io, mem::size_of, path::Path, ptr::NonNull};

use cherenkov_oracle::F32Image;
use objc2_core_foundation::{
    CFBoolean, CFData, CFDictionary, CFRetained, CFType, CGPoint, CGRect, CGSize,
};
use objc2_core_graphics::{
    CGColorSpace, CGContext, CGImage, CGImageAlphaInfo, CGImageByteOrderInfo, CGImageComponentInfo,
    kCGColorSpaceExtendedLinearDisplayP3,
};
use objc2_image_io::{
    CGImageSource, CGImageSourceStatus, kCGImageSourceDecodeRequest, kCGImageSourceDecodeToHDR,
    kCGImageSourceShouldAllowFloat, kCGImageSourceShouldCache,
    kCGImageSourceShouldCacheImmediately,
};

use super::Error;

unsafe extern "C" {
    // This public CoreGraphics function has no objc2 binding.
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: Option<&CGColorSpace>,
        bitmap_info: u32,
    ) -> *mut CGContext;
}

pub fn load(path: &Path) -> Result<F32Image, Error> {
    let bytes = std::fs::read(path)?;
    validate_png_metadata(&bytes)?;
    let data = CFData::from_bytes(&bytes);
    // SAFETY: No source options are supplied; data is owned for the decode.
    let source = unsafe { CGImageSource::with_data(&data, None) }
        .ok_or_else(|| io::Error::other(format!("cannot read image: {}", path.display())))?;
    // SAFETY: These are the system's immutable Image I/O option constants.
    let (keys, values): ([_; 4], [&CFType; 4]) = unsafe {
        (
            [
                kCGImageSourceShouldAllowFloat,
                kCGImageSourceDecodeRequest,
                kCGImageSourceShouldCache,
                kCGImageSourceShouldCacheImmediately,
            ],
            [
                CFBoolean::new(true),
                kCGImageSourceDecodeToHDR,
                CFBoolean::new(true),
                CFBoolean::new(true),
            ],
        )
    };
    let options = CFDictionary::from_slices(&keys, &values);
    // SAFETY: Image I/O specifies a CFBoolean and a CFString for these keys.
    let image = unsafe { source.image_at_index(0, Some(options.as_opaque())) }
        .ok_or_else(|| io::Error::other(format!("cannot decode image: {}", path.display())))?;
    // SAFETY: Index zero was just decoded from this retained source.
    if unsafe { source.status_at_index(0) } != CGImageSourceStatus::StatusComplete {
        return Err(io::Error::other(format!("incomplete image: {}", path.display())).into());
    }
    rasterize(&image)
}

fn validate_png_metadata(mut bytes: &[u8]) -> Result<(), Error> {
    let mut decoder = png::StreamingDecoder::new();
    decoder.set_skip_ancillary_crc_failures(false);
    while !bytes.is_empty() {
        // None skips pixel decompression; Image I/O owns the actual decode.
        let (consumed, event) = decoder.update(bytes, None)?;
        bytes = &bytes[consumed..];
        match event {
            png::Decoded::ChunkComplete(png::chunk::iCCP) => {
                let profile = decoder
                    .info()
                    .and_then(|info| info.icc_profile.as_deref())
                    .ok_or_else(|| io::Error::other("unreadable PNG ICC profile"))?;
                let data = CFData::from_bytes(profile);
                // SAFETY: The CFType supplied to this API is owned CFData.
                let _space = unsafe { CGColorSpace::with_icc_data(Some(&data)) }
                    .ok_or_else(|| io::Error::other("invalid or unsupported PNG ICC profile"))?;
            }
            png::Decoded::BadAncillaryChunk(kind) => {
                return Err(io::Error::other(format!("invalid PNG metadata: {kind:?}")).into());
            }
            png::Decoded::ChunkComplete(png::chunk::IEND) => return Ok(()),
            _ => {}
        }
    }
    Err(io::Error::new(
        io::ErrorKind::UnexpectedEof,
        "PNG has no complete IEND chunk",
    )
    .into())
}

fn rasterize(image: &CGImage) -> Result<F32Image, Error> {
    let width = CGImage::width(Some(image));
    let height = CGImage::height(Some(image));
    let count = width
        .checked_mul(height)
        .filter(|&count| count > 0)
        .ok_or_else(|| io::Error::other("invalid screenshot dimensions"))?;
    let row_bytes = width
        .checked_mul(size_of::<[f32; 4]>())
        .ok_or_else(|| io::Error::other("screenshot row size overflow"))?;
    let output_width = u32::try_from(width)?;
    let output_height = u32::try_from(height)?;
    let mut pixels = vec![[0.0f32; 4]; count];
    // SAFETY: The system's named color-space constant is immutable.
    let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceExtendedLinearDisplayP3 }))
        .ok_or_else(|| io::Error::other("extended linear Display P3 is unavailable"))?;
    let info = CGImageAlphaInfo::PremultipliedLast.0
        | CGImageComponentInfo::Float.0
        | CGImageByteOrderInfo::Order32Little.0;
    // SAFETY: The RGBA-f32 allocation is width*height pixels and remains
    // unchanged in size until the synchronous draw and context drop finish.
    let raw = unsafe {
        CGBitmapContextCreate(
            pixels.as_mut_ptr().cast(),
            width,
            height,
            32,
            row_bytes,
            Some(&space),
            info,
        )
    };
    let raw = NonNull::new(raw)
        .ok_or_else(|| io::Error::other("cannot create a floating-point P3 bitmap context"))?;
    // SAFETY: CGBitmapContextCreate returned a +1 CoreFoundation object.
    let context = unsafe { CFRetained::from_raw(raw) };
    CGContext::draw_image(
        Some(&context),
        CGRect::new(
            CGPoint::new(0.0, 0.0),
            CGSize::new(f64::from(output_width), f64::from(output_height)),
        ),
        Some(image),
    );
    drop(context);
    Ok(F32Image {
        width: output_width,
        height: output_height,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use objc2_core_foundation::CFString;
    use objc2_core_graphics::{
        CGBitmapContextCreateImage, kCGColorSpaceDisplayP3, kCGColorSpaceSRGB,
    };

    use super::*;

    fn png(samples: [u16; 4], space: &CFString) -> Vec<u8> {
        let space = CGColorSpace::with_name(Some(space)).unwrap();
        let profile = CGColorSpace::icc_data(Some(&space)).unwrap().to_vec();
        png_with_profile(samples, profile)
    }

    fn png_with_profile(samples: [u16; 4], profile: Vec<u8>) -> Vec<u8> {
        let mut info = png::Info::with_size(1, 1);
        info.bit_depth = png::BitDepth::Sixteen;
        info.color_type = png::ColorType::Rgba;
        info.icc_profile = Some(Cow::Owned(profile));
        encode_png(samples, info)
    }

    fn encode_png(samples: [u16; 4], info: png::Info<'_>) -> Vec<u8> {
        let cicp = info.coding_independent_code_points;
        let mut bytes = Vec::new();
        {
            let mut writer = png::Encoder::with_info(&mut bytes, info)
                .unwrap()
                .write_header()
                .unwrap();
            if let Some(cicp) = cicp {
                writer
                    .write_chunk(
                        png::chunk::cICP,
                        &[
                            cicp.color_primaries,
                            cicp.transfer_function,
                            cicp.matrix_coefficients,
                            u8::from(cicp.is_video_full_range_image),
                        ],
                    )
                    .unwrap();
            }
            let samples: Vec<_> = samples.into_iter().flat_map(u16::to_be_bytes).collect();
            writer.write_image_data(&samples).unwrap();
            writer.finish().unwrap();
        }
        bytes
    }

    fn decode_png(bytes: &[u8]) -> F32Image {
        load_png(bytes).unwrap()
    }

    fn load_png(bytes: &[u8]) -> Result<F32Image, Error> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.png");
        std::fs::write(&path, bytes).unwrap();
        load(&path)
    }

    #[test]
    fn tagged_p3_red_is_not_clipped_through_srgb() {
        // SAFETY: The system color-space constant is immutable.
        let bytes = png([65535, 0, 0, 65535], unsafe { kCGColorSpaceDisplayP3 });
        let image = decode_png(&bytes);
        let actual = image.pixels[0];
        assert!((actual[0] - 1.0).abs() < 1e-4, "{actual:?}");
        assert!(
            actual[1].abs() < 1e-4 && actual[2].abs() < 1e-4,
            "{actual:?}"
        );
    }

    #[test]
    fn sixteen_bit_srgb_precision_survives_decode() {
        // SAFETY: The system color-space constant is immutable.
        let bytes = png([32768, 32768, 32768, 65535], unsafe { kCGColorSpaceSRGB });
        let image = decode_png(&bytes);
        let expected = cherenkov_oracle::color::srgb_decode(32768.0 / 65535.0);
        for channel in &image.pixels[0][..3] {
            assert!((f64::from(*channel) - expected).abs() < 1e-4, "{channel}");
        }
    }

    #[test]
    fn alpha_premultiplication_happens_in_linear_light() {
        // SAFETY: The system color-space constant is immutable.
        let bytes = png([32768, 32768, 32768, 32768], unsafe { kCGColorSpaceSRGB });
        let image = decode_png(&bytes);
        let alpha = 32768.0 / 65535.0;
        let expected = cherenkov_oracle::color::srgb_decode(alpha) * alpha;
        for channel in &image.pixels[0][..3] {
            assert!((f64::from(*channel) - expected).abs() < 1e-4, "{channel}");
        }
    }

    #[test]
    fn hdr_png_is_not_decoded_as_sdr() {
        let mut info = png::Info::with_size(1, 1);
        info.bit_depth = png::BitDepth::Sixteen;
        info.color_type = png::ColorType::Rgba;
        // ITU-T H.273: BT.2020 primaries, ST 2084 PQ, full-range RGB.
        let cicp = png::CodingIndependentCodePoints {
            color_primaries: 9,
            transfer_function: 16,
            matrix_coefficients: 0,
            is_video_full_range_image: true,
        };
        info.coding_independent_code_points = Some(cicp);
        let bytes = encode_png([65535; 4], info);
        let reader = png::Decoder::new(std::io::Cursor::new(&bytes))
            .read_info()
            .unwrap();
        assert_eq!(reader.info().coding_independent_code_points, Some(cicp));
        let image = decode_png(&bytes);
        assert!(image.pixels[0][0] > 1.0, "{:?}", image.pixels[0]);
    }

    #[test]
    fn floating_point_hdr_survives_the_p3_bitmap() {
        let mut source = [[2.0f32, 1.25, 0.5, 1.0]];
        // SAFETY: The system color-space constant is immutable.
        let space =
            CGColorSpace::with_name(Some(unsafe { kCGColorSpaceExtendedLinearDisplayP3 })).unwrap();
        // SAFETY: Source is one RGBA-f32 pixel and outlives the context/image.
        let raw = unsafe {
            CGBitmapContextCreate(
                source.as_mut_ptr().cast(),
                1,
                1,
                32,
                size_of::<[f32; 4]>(),
                Some(&space),
                CGImageAlphaInfo::PremultipliedLast.0
                    | CGImageComponentInfo::Float.0
                    | CGImageByteOrderInfo::Order32Little.0,
            )
        };
        // SAFETY: The create function returns a +1 object, kept with source.
        let context = unsafe { CFRetained::from_raw(NonNull::new(raw).unwrap()) };
        let image = CGBitmapContextCreateImage(Some(&context)).unwrap();
        let actual = rasterize(&image).unwrap();
        for (actual, expected) in actual.pixels[0].iter().zip(source[0]) {
            assert!((actual - expected).abs() < 1e-4, "{actual} != {expected}");
        }
    }

    #[test]
    fn unreadable_and_malformed_images_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.png");
        assert!(load(&path).is_err());
        std::fs::write(&path, b"not a PNG").unwrap();
        assert!(load(&path).is_err());
        // SAFETY: The system color-space constant is immutable.
        let bytes = png([0, 0, 0, 65535], unsafe { kCGColorSpaceSRGB });
        assert!(load_png(&bytes[..bytes.len() - 1]).is_err());
    }

    #[test]
    fn an_invalid_icc_profile_is_not_silently_replaced() {
        let bytes = png_with_profile([0, 0, 0, 65535], b"invalid ICC profile".to_vec());
        let reader = png::Decoder::new(std::io::Cursor::new(&bytes))
            .read_info()
            .unwrap();
        assert_eq!(
            reader.info().icc_profile.as_deref(),
            Some(b"invalid ICC profile".as_slice())
        );
        assert!(load_png(&bytes).is_err());
    }
}
